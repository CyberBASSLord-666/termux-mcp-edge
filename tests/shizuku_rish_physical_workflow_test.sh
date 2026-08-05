#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
WORKFLOW="$ROOT/.github/workflows/shizuku-rish-physical-qualification.yml"
RESOLVER="$ROOT/scripts/resolve_shizuku_rish_candidate.sh"
PACKAGER="$ROOT/scripts/package_shizuku_rish_test_artifact.sh"
CONTROLLER="$ROOT/scripts/shizuku_rish_physical_controller.sh"
DEVICE_GATE="$ROOT/scripts/termux_rish_physical_gate.sh"
VALIDATOR="$ROOT/scripts/validate_rish_physical_identity_evidence.py"
ARTIFACT_SCHEMA="$ROOT/docs/android-rish-development-artifact-schema-v1.json"
EVIDENCE_SCHEMA="$ROOT/docs/android-rish-physical-identity-evidence-schema-v1.json"
POLICY_SCHEMA="$ROOT/docs/android-rish-physical-identity-policy-schema-v1.json"
POLICY="$ROOT/docs/android-rish-physical-identity-policy-v1.json"
RUN_SELECTOR="$ROOT/scripts/latest_workflow_run.jq"
CI_WORKFLOW="$ROOT/.github/workflows/ci.yml"
SECURITY_WORKFLOW="$ROOT/.github/workflows/security.yml"
ANDROID_WORKFLOW="$ROOT/.github/workflows/android-cross-compile.yml"

fail() {
  printf 'Shizuku/rish physical workflow contract failed: %s\n' "$1" >&2
  exit 1
}

for file in \
  "$WORKFLOW" "$RESOLVER" "$PACKAGER" "$CONTROLLER" "$DEVICE_GATE" \
  "$VALIDATOR" "$ARTIFACT_SCHEMA" "$EVIDENCE_SCHEMA" "$POLICY_SCHEMA" \
  "$POLICY" "$RUN_SELECTOR"
do
  [[ -f "$file" && ! -L "$file" ]] || fail "required file missing or linked: $file"
done
for workflow in "$CI_WORKFLOW" "$SECURITY_WORKFLOW" "$ANDROID_WORKFLOW"; do
  [[ -f "$workflow" && ! -L "$workflow" ]] \
    || fail "companion workflow missing or linked: $workflow"
  grep -Fq -- '"scripts/resolve_shizuku_rish_candidate.sh"' "$workflow" \
    || fail "candidate resolver absent from companion path filter: $workflow"
done
grep -Fq -- '"tests/**"' "$CI_WORKFLOW" \
  || fail CI_test_path_filter_missing
grep -Fq -- '"docs/SHIZUKU_RISH_PHYSICAL_WORKFLOW.md"' "$CI_WORKFLOW" \
  || fail CI_physical_doc_path_filter_missing
grep -Fq -- '"tests/**"' "$SECURITY_WORKFLOW" \
  || fail Security_test_path_filter_missing
grep -Fq -- '"docs/SHIZUKU_RISH_PHYSICAL_WORKFLOW.md"' "$SECURITY_WORKFLOW" \
  || fail Security_physical_doc_path_filter_missing
grep -Fq -- '"tests/resolve_shizuku_rish_candidate_test.sh"' "$ANDROID_WORKFLOW" \
  || fail Android_resolver_test_path_filter_missing
grep -Fq -- '"tests/fixtures/rish_candidate_mock_curl.py"' "$ANDROID_WORKFLOW" \
  || fail Android_resolver_fixture_path_filter_missing
for physical_path in \
  '"tests/package_shizuku_rish_test_artifact_test.sh"' \
  '"tests/rish_physical_identity_evidence_test.sh"' \
  '"tests/shizuku_rish_physical_controller_test.sh"' \
  '"tests/shizuku_rish_physical_workflow_test.sh"' \
  '"tests/termux_rish_physical_gate_test.sh"' \
  '"docs/SHIZUKU_RISH_PHYSICAL_WORKFLOW.md"'
do
  grep -Fq -- "$physical_path" "$ANDROID_WORKFLOW" \
    || fail "Android physical path filter missing: $physical_path"
done
grep -Fq -- 'bash tests/resolve_shizuku_rish_candidate_test.sh' "$CI_WORKFLOW" \
  || fail resolver_behavior_test_not_wired_to_CI
grep -Fq -- 'bash tests/shizuku_rish_physical_controller_test.sh' "$CI_WORKFLOW" \
  || fail physical_controller_test_not_wired_to_CI
grep -Fq -- 'bash tests/termux_rish_physical_gate_test.sh' "$CI_WORKFLOW" \
  || fail physical_device_gate_test_not_wired_to_CI

bash -n "$RESOLVER" "$PACKAGER" "$CONTROLLER" "$DEVICE_GATE"
python3 - "$VALIDATOR" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
compile(path.read_text(encoding="utf-8"), str(path), "exec")
PY

python3 - "$WORKFLOW" "$RESOLVER" <<'PY'
import pathlib
import subprocess
import sys

try:
    import yaml
except ImportError as error:
    raise SystemExit(f"PyYAML is required to parse the workflow: {error}")

workflow_path = pathlib.Path(sys.argv[1])
resolver_path = pathlib.Path(sys.argv[2])
try:
    workflow_text = workflow_path.read_text(encoding="utf-8")
    resolver_text = resolver_path.read_text(encoding="utf-8")
    document = yaml.safe_load(workflow_text)
except (OSError, UnicodeError, yaml.YAMLError) as error:
    raise SystemExit(f"workflow contract input did not parse: {error}")

if not isinstance(document, dict):
    raise SystemExit("workflow root is not a mapping")
if document.get("name") != "Android Rish Physical Identity":
    raise SystemExit("workflow name changed")

# PyYAML 1.1 treats the unquoted GitHub key `on` as boolean true.
trigger = document.get("on", document.get(True))
expected_trigger = {
    "workflow_dispatch": {
        "inputs": {
            "expected_commit": {
                "description": "Exact open same-repository pull-request head SHA",
                "required": True,
                "type": "string",
            },
            "pull_request_number": {
                "description": "Open same-repository pull request targeting main",
                "required": True,
                "type": "string",
            },
        }
    }
}
if trigger != expected_trigger:
    raise SystemExit(f"workflow must retain the exact manual-only trigger: {trigger!r}")
if document.get("permissions") != {}:
    raise SystemExit("top-level permissions must remain empty")
if document.get("concurrency") != {
    "group": "android-rish-physical-controller-v1",
    "cancel-in-progress": False,
}:
    raise SystemExit("physical controller concurrency contract changed")

jobs = document.get("jobs")
expected_job_names = [
    "preflight-resolve",
    "candidate-build",
    "candidate-review",
    "physical-gate",
    "validate-evidence",
    "final-review",
]
if not isinstance(jobs, dict) or list(jobs) != expected_job_names:
    raise SystemExit("workflow must keep the exact six ordered jobs")

expected_job_contracts = {
    "preflight-resolve": {
        "runs-on": "ubuntu-24.04",
        "timeout-minutes": 30,
        "permissions": {
            "actions": "read",
            "contents": "read",
            "pull-requests": "read",
        },
    },
    "candidate-build": {
        "runs-on": "ubuntu-24.04",
        "timeout-minutes": 30,
        "permissions": {},
        "needs": "preflight-resolve",
    },
    "candidate-review": {
        "runs-on": "ubuntu-24.04",
        "timeout-minutes": 15,
        "permissions": {
            "actions": "read",
            "contents": "read",
            "pull-requests": "read",
        },
        "needs": ["preflight-resolve", "candidate-build"],
    },
    "physical-gate": {
        "runs-on": ["self-hosted", "linux", "termux-rish-controller"],
        "timeout-minutes": 30,
        "permissions": {
            "actions": "read",
            "contents": "read",
            "pull-requests": "read",
        },
        "environment": {
            "name": "android-rish-physical-development",
            "deployment": False,
        },
        "needs": "candidate-review",
    },
    "validate-evidence": {
        "runs-on": "ubuntu-24.04",
        "timeout-minutes": 15,
        "permissions": {
            "actions": "read",
            "contents": "read",
            "pull-requests": "read",
        },
        "needs": ["candidate-review", "physical-gate"],
    },
    "final-review": {
        "runs-on": "ubuntu-24.04",
        "timeout-minutes": 15,
        "permissions": {
            "actions": "read",
            "contents": "read",
            "pull-requests": "read",
        },
        "environment": {
            "name": "android-rish-physical-final-review",
            "deployment": False,
        },
        "needs": ["candidate-review", "physical-gate", "validate-evidence"],
    },
}
for job_name, expected in expected_job_contracts.items():
    job = jobs[job_name]
    for key, value in expected.items():
        if job.get(key) != value:
            raise SystemExit(f"{job_name} {key} contract changed")
    if "continue-on-error" in job or "if" in job:
        raise SystemExit(f"{job_name} may not bypass or ignore fail-closed ordering")
    if job_name == "candidate-build":
        if job.get("permissions") != {}:
            raise SystemExit("candidate build must remain permissionless")
    else:
        if set(job.get("permissions", {})) != {
            "actions",
            "contents",
            "pull-requests",
        }:
            raise SystemExit(f"{job_name} gained a permission family")
        if any(value != "read" for value in job.get("permissions", {}).values()):
            raise SystemExit(f"{job_name} gained write authority")

expected_steps = {
    "preflight-resolve": [
        "Validate closed dispatch identity",
        "Checkout trusted workflow definition",
        "Record trusted identities",
        "Resolve exact candidate and companion runs",
        "Requery current workflow invocation",
    ],
    "candidate-build": [
        "Validate permissionless candidate boundary",
        "Checkout trusted build helpers without credentials",
        "Checkout exact candidate without credentials",
        "Install pinned Rust toolchain",
        "Install Android NDK",
        "Verify and cross-compile exact candidate",
        "Package closed development artifact",
        "Record candidate identity",
        "Upload closed development artifact",
    ],
    "candidate-review": [
        "Checkout trusted reconciliation definition",
        "Requery exact candidate and companion runs after build",
        "Reconcile uploaded candidate artifact identity",
        "Record protected physical review request",
    ],
    "physical-gate": [
        "Validate protected controller boundary",
        "Checkout trusted controller definition only",
        "Verify trusted controller checkout",
        "Requery exact candidate and companion runs",
        "Download exact hosted candidate artifact",
        "Validate downloaded candidate identity",
        "Execute trusted physical identity controller",
        "Upload sanitized physical evidence",
        "Reconcile uploaded physical evidence identity",
        "Record protected final review request",
    ],
    "validate-evidence": [
        "Checkout trusted validator definition",
        "Requery exact candidate and companion runs",
        "Download exact sanitized evidence",
        "Download exact candidate artifact for reconciliation",
        "Validate closed evidence and artifact binding",
    ],
    "final-review": [
        "Validate final protected boundary",
        "Checkout trusted final validator definition",
        "Requery exact identities after final approval",
        "Download exact sanitized evidence after approval",
        "Download exact candidate artifact after approval",
        "Repeat closed validation after final approval",
        "Final candidate state requery before evidence upload",
        "Upload final development-only identity evidence",
        "Reconcile uploaded final evidence identity",
        "Record non-release qualification boundary",
    ],
}

allowed_actions = {
    "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
    "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c",
    "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
    "dtolnay/rust-toolchain@29eef336d9b2848a0b548edc03f92a220660cdb8",
    "nttld/setup-ndk@ed92fe6cadad69be94a966a7ee3271275e62f779",
}
action_counts = {action: 0 for action in allowed_actions}
candidate_checkout_jobs = []
resolver_jobs = []
validator_jobs = []
for job_name, job in jobs.items():
    steps = job.get("steps")
    if not isinstance(steps, list) or not steps:
        raise SystemExit(f"{job_name} has no steps")
    names = [step.get("name") for step in steps]
    if names != expected_steps[job_name] or len(names) != len(set(names)):
        raise SystemExit(f"{job_name} step ordering changed")
    for index, step in enumerate(steps):
        if not isinstance(step, dict) or not isinstance(step.get("name"), str):
            raise SystemExit(f"{job_name} step {index} is invalid")
        if ("run" in step) == ("uses" in step):
            raise SystemExit(f"{job_name} step {index} must have one execution kind")
        if any(key in step for key in ("continue-on-error", "if", "shell")):
            raise SystemExit(f"{job_name} step {step['name']} may not bypass policy")
        if "uses" in step:
            action = step["uses"]
            if action not in allowed_actions:
                raise SystemExit(f"unapproved action: {action}")
            action_counts[action] += 1
            with_values = step.get("with", {})
            if action.startswith("actions/checkout@"):
                if with_values.get("persist-credentials") is not False:
                    raise SystemExit("checkout credentials must remain disabled")
                if with_values.get("ref") != "${{ github.workflow_sha }}":
                    raise SystemExit("trusted checkout is not bound to workflow SHA")
            if action.startswith("actions/download-artifact@"):
                required = {
                    "artifact-ids",
                    "path",
                    "github-token",
                    "repository",
                    "run-id",
                    "digest-mismatch",
                }
                if not required.issubset(with_values):
                    raise SystemExit("download is missing exact identity fields")
                if with_values["repository"] != "${{ github.repository }}":
                    raise SystemExit("download repository is not fixed")
                if with_values["run-id"] != "${{ github.run_id }}":
                    raise SystemExit("download is not bound to the current run")
                if with_values["digest-mismatch"] != "error":
                    raise SystemExit("download digest mismatch must fail")
            if action.startswith("actions/upload-artifact@"):
                for key, expected in (
                    ("if-no-files-found", "error"),
                    ("include-hidden-files", False),
                    ("compression-level", 0),
                    ("overwrite", False),
                    ("retention-days", 30),
                ):
                    if with_values.get(key) != expected:
                        raise SystemExit(f"upload option changed: {key}")
        else:
            script = step["run"]
            if not isinstance(script, str):
                raise SystemExit(f"{job_name} step {step['name']} has no shell text")
            result = subprocess.run(
                ["bash", "-n"],
                input=script,
                text=True,
                capture_output=True,
                check=False,
            )
            if result.returncode != 0:
                raise SystemExit(
                    f"embedded Bash syntax failed for {step['name']}: {result.stderr}"
                )
            if "resolve_shizuku_rish_candidate.sh" in script:
                resolver_jobs.append(job_name)
            if "validate_rish_physical_identity_evidence.py" in script:
                validator_jobs.append(job_name)
            if step["name"] == "Checkout exact candidate without credentials":
                candidate_checkout_jobs.append(job_name)

if candidate_checkout_jobs != ["candidate-build"]:
    raise SystemExit("candidate source may be fetched only by the permissionless build")
if resolver_jobs != [
    "preflight-resolve",
    "candidate-review",
    "physical-gate",
    "validate-evidence",
    "final-review",
    "final-review",
]:
    raise SystemExit("candidate/companion run resolution is not repeated at every boundary")
if validator_jobs != ["validate-evidence", "final-review"]:
    raise SystemExit("closed evidence validation must run before and after final approval")

expected_action_counts = {
    "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1": 5,
    "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c": 5,
    "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a": 3,
    "dtolnay/rust-toolchain@29eef336d9b2848a0b548edc03f92a220660cdb8": 1,
    "nttld/setup-ndk@ed92fe6cadad69be94a966a7ee3271275e62f779": 1,
}
if action_counts != expected_action_counts:
    raise SystemExit(f"action count or pin changed: {action_counts!r}")

candidate_job = jobs["candidate-build"]
for step in candidate_job["steps"]:
    if str(step.get("uses", "")).startswith("actions/checkout@"):
        raise SystemExit("permissionless candidate build may not use token-aware checkout")
    if {"GH_TOKEN", "GITHUB_TOKEN"} & set(step.get("env", {})):
        raise SystemExit("permissionless candidate build received a GitHub token")
    script = step.get("run", "")
    if "${{ github.token }}" in script or "${{ secrets." in script:
        raise SystemExit("permissionless candidate build references a secret context")

candidate_text = "\n".join(
    step.get("run", "")
    for step in candidate_job["steps"]
    if isinstance(step, dict)
)
for checkout_name in (
    "Checkout trusted build helpers without credentials",
    "Checkout exact candidate without credentials",
):
    checkout_step = next(
        step for step in candidate_job["steps"] if step["name"] == checkout_name
    )
    checkout_env = checkout_step.get("env", {})
    if checkout_env.get("GIT_CONFIG_GLOBAL") != "/dev/null":
        raise SystemExit("permissionless fetch may not load global git credentials")
    if checkout_env.get("GIT_CONFIG_NOSYSTEM") != "1":
        raise SystemExit("permissionless fetch may not load system git credentials")
    if checkout_env.get("GIT_TERMINAL_PROMPT") != "0":
        raise SystemExit("permissionless fetch may not prompt for credentials")
for marker in (
    "https://github.com/CyberBASSLord-666/termux-mcp-edge.git",
    "refs/heads/$EXPECTED_HEAD_BRANCH",
    "-c credential.helper=",
    "-c http.extraheader=",
    'test -z "${GH_TOKEN:-}"',
    'test -z "${GITHUB_TOKEN:-}"',
    "-u GITHUB_ENV",
    "-u GITHUB_OUTPUT",
    "-u GITHUB_PATH",
    "-u GITHUB_STEP_SUMMARY",
):
    if marker not in candidate_text:
        raise SystemExit(f"permissionless candidate boundary marker missing: {marker}")
if "resolve_shizuku_rish_candidate.sh" in candidate_text:
    raise SystemExit("candidate build may not resolve PR state or receive its token")

for trusted_job_name in ("preflight-resolve", "candidate-review"):
    trusted_text = "\n".join(
        step.get("run", "")
        for step in jobs[trusted_job_name]["steps"]
        if isinstance(step, dict)
    )
    if "cargo build" in trusted_text or "refs/heads/$EXPECTED_HEAD_BRANCH" in trusted_text:
        raise SystemExit(f"{trusted_job_name} may not fetch or build candidate source")

for forbidden in (
    "pull_request_target",
    "schedule:",
    "secrets.",
    "permissions: write",
    "actions: write",
    "contents: write",
    "packages: write",
    "id-token: write",
    "gh release",
    "/releases",
    "package_android_artifact.sh",
):
    if forbidden in workflow_text:
        raise SystemExit(f"forbidden workflow authority or release coupling: {forbidden}")

physical_text = "\n".join(
    step.get("run", "")
    for step in jobs["physical-gate"]["steps"]
    if isinstance(step, dict)
)
if "cargo " in physical_text or "candidate/scripts/" in physical_text:
    raise SystemExit("physical controller job may not build or execute candidate source")
for marker in (
    "trusted/scripts/shizuku_rish_physical_controller.sh",
    "trusted/scripts/termux_rish_physical_gate.sh",
    "--controller-challenge-file",
    "--workflow-definition-sha256",
    "--workflow-run-id",
    "--workflow-run-attempt",
    "--ci-run-id",
    "--security-run-id",
    "--android-run-id",
    "head -c 32 /dev/urandom",
):
    if marker not in physical_text:
        raise SystemExit(f"physical identity binding marker missing: {marker}")

if workflow_text.count("outputs.artifact-digest") != 6:
    raise SystemExit("artifact action digests are not propagated and reconciled")
if workflow_text.count(".digest == $digest") != 6:
    raise SystemExit("uploaded/downloaded artifact API digests are not reconciled")
if workflow_text.count("pull-requests: read") != 5:
    raise SystemExit("every candidate resolver job requires pull-requests:read")

for marker in (
    'and .draft == false',
    'and .head.repo.full_name == $repository',
    'and .base.repo.full_name == $repository',
    'and .base.ref == "main"',
    'pulls/$PULL_REQUEST/reviews?per_page=100',
    '--max-filesize 16777216',
    'and length < 100',
    '.state == "APPROVED"',
    'and .commit_id == $commit',
    '"MEMBER"',
    '"OWNER"',
    '"COLLABORATOR"',
    'def trusted_approvals($allow_author):',
    'ANDROID_RISH_PHYSICAL_SOLO_OPERATOR',
    'reviewed-v1',
    'and .event == "pull_request"',
    'and .head_sha == $commit',
    'and .status == "completed"',
    'and .conclusion == "success"',
    'and .run_attempt == 1',
    'CI_RUN_ID="$(resolve_run CI ci.yml',
    'SECURITY_RUN_ID="$(resolve_run Security security.yml',
    '"Android Cross Compile"',
):
    if marker not in resolver_text:
        raise SystemExit(f"candidate resolver marker missing: {marker}")
PY

jq -e '
  .type == "object"
  and .additionalProperties == false
  and .properties.artifactClass.const == "android_rish_development_only_v1"
  and .properties.releaseEligible.const == false
  and .properties.productionControlQualified.const == false
  and .properties.artifactName.const == "termux-mcp-server-aarch64-linux-android-android-rish-development"
  and .properties.posture.const == "android-rish-development"
  and .properties.features.const == ["android-rish"]
  and .properties.target.const == "aarch64-linux-android"
' "$ARTIFACT_SCHEMA" >/dev/null \
  || fail development_artifact_schema_boundary_changed

jq -e '
  .releaseEligible == false
  and .productionControlQualified == false
  and .qualificationClass == "physical_shizuku_rish_identity_development_v1"
  and .scope == "s2_5_uid_probe_only"
  and .evidenceFileName == "android-rish-physical-identity-evidence-v1.json"
  and .workflow.name == "Android Rish Physical Identity"
  and .workflow.runAttempt == 1
  and .workflow.event == "workflow_dispatch"
  and .workflow.protectedEnvironment == "android-rish-physical-development"
  and .artifact.artifactName == "termux-mcp-server-aarch64-linux-android-android-rish-development"
  and .artifact.posture == "android-rish-development"
  and .artifact.features == ["android-rish"]
  and .artifact.target == "aarch64-linux-android"
  and .environment.requiredShizukuStartMode == "adb"
  and .backend.rootAccepted == false
  and .backend.arbitraryShell == false
  and .backend.mutationReady == false
  and .validation.scenarioIds == [
    "controller_offline_posture_pre_candidate",
    "trusted_direct_rish_probe_pre_candidate",
    "runtime_disabled_tool_absent",
    "candidate_mcp_status_uid_2000",
    "extra_arguments_rejected",
    "unknown_shell_rejected",
    "dex_tamper_rejected",
    "dex_mode_rejected",
    "dex_symlink_rejected",
    "all_mutation_gates_disabled",
    "trusted_direct_rish_probe_post_candidate",
    "controller_offline_posture_post_candidate",
    "bounded_test_fixture_cleanup",
    "device_slot_quarantined_after_candidate"
  ]
  and .claimBoundary.sameUidPersistenceExcluded == false
  and .claimBoundary.continuousNetworkIsolation == false
  and .claimBoundary.adversarialNetworkIsolation == false
  and .claimBoundary.productionControl == false
  and .cleanup == {
    candidateProcessGroupStopped: true,
    portReleased: true,
    deviceFixtureStateRemoved: true,
    controllerTransportRemoved: true,
    deviceSlotQuarantinedAfterCandidate: true
  }
' "$POLICY" >/dev/null || fail physical_policy_boundary_changed

for marker in \
  'controllerOfflinePosturePreCandidate' \
  'trustedDirectRishProbePreCandidate' \
  'trustedDirectRishProbePostCandidate' \
  'controllerOfflinePosturePostCandidate' \
  'controller_offline_posture_pre_candidate' \
  'trusted_direct_rish_probe_pre_candidate' \
  'candidate_mcp_status_uid_2000' \
  'trusted_direct_rish_probe_post_candidate' \
  'controller_offline_posture_post_candidate' \
  'bounded_test_fixture_cleanup' \
  'device_slot_quarantined_after_candidate' \
  'candidateProcessGroupStopped' \
  'sameUidPersistenceExcluded' \
  'continuousNetworkIsolation' \
  'adversarialNetworkIsolation' \
  'shizukuStartModeObserved'
do
  grep -Fq -- "$marker" "$CONTROLLER" "$DEVICE_GATE" "$VALIDATOR" "$EVIDENCE_SCHEMA" \
    || fail "final physical identity marker missing: $marker"
done

printf 'Shizuku/rish physical workflow contract tests passed\n'
