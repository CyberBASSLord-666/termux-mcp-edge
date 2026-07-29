#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
WORKFLOW="$ROOT/.github/workflows/stage-release-assets.yml"
ANDROID_WORKFLOW="$ROOT/.github/workflows/android-cross-compile.yml"
CI_WORKFLOW="$ROOT/.github/workflows/ci.yml"

fail() {
  printf 'release staging workflow contract failed: %s\n' "$1" >&2
  exit 1
}

assert_contains() {
  local value="$1"
  grep -Fq -- "$value" "$WORKFLOW" || fail "missing marker: $value"
}

[[ -f "$WORKFLOW" && ! -L "$WORKFLOW" ]] || fail workflow_missing_or_linked
[[ "$(grep -Ec '^[[:space:]]+workflow_dispatch:$' "$WORKFLOW")" -eq 1 ]] \
  || fail workflow_must_be_manual_only
if grep -Eq '^[[:space:]]+(push|pull_request|schedule):' "$WORKFLOW"; then
  fail automatic_staging_trigger_present
fi

assert_contains '  actions: read'
assert_contains '  contents: read'
assert_contains '      name: release-qualification'
assert_contains '      deployment: false'
assert_contains 'RELEASE_QUALIFICATION_PROTECTED'
assert_contains 'required-reviewer-main-only-v1'
assert_contains 'refs/heads/main'
assert_contains 'RUN_ATTEMPT: ${{ github.run_attempt }}'
assert_contains 'test "$RUN_ATTEMPT" = 1'
assert_contains '.object.sha == $sha'
assert_contains '.run_attempt == 1'
assert_contains 'include "latest_workflow_run"'
assert_contains 'complete_workflow_run_page'
assert_contains '$latest.id == $run_id'
assert_contains '.expired == false'
assert_contains '.total_count == 9'
assert_contains '.total_count == 1'
assert_contains 'artifact-ids:'
assert_contains 'github-token: ${{ github.token }}'
assert_contains 'digest-mismatch: error'
assert_contains 'archive: false'
assert_contains 'retention-days: 30'
assert_contains 'staged_not_released'
assert_contains 'Release eligible: `false`'
assert_contains 'automated-qualification-v1.json'
assert_contains 'automated-native-deployment-v1.json'
assert_contains 'termux-native-aggregate-evidence-v4.json'
assert_contains '--automated-qualification "$qualification"'
assert_contains '--deployment-evidence "$deployment"'
assert_contains '--qualification-run-id "$QUALIFICATION_RUN_ID"'
assert_contains 'actions/workflows/automated-release-qualification.yml/runs?branch=main&event=workflow_run&head_sha=$EXPECTED_COMMIT&per_page=100'
assert_contains 'Qualify Android run $ANDROID_RUN_ID at $EXPECTED_COMMIT'
assert_contains '.display_title == $title'
assert_contains '.path == ".github/workflows/automated-release-qualification.yml"'
assert_contains '.event == "workflow_run"'
assert_contains 'qualification_artifact_digest'
assert_contains 'android_artifact_set_sha256'
assert_contains 'runtime_artifact_id'
assert_contains 'runtime_artifact_digest'
assert_contains 'termux-mcp-qualified-runtime-snapshot'
assert_contains 'termux-qualified-runtime-image-v1.tar.gz'
assert_contains 'termux-runtime-package-lock-v1.json'
assert_contains 'termux-runtime-snapshot-v1.json'
assert_contains 'termux-runtime-snapshot-replay-v1.json'
assert_contains '--runtime-archive "$runtime_archive"'
assert_contains '--runtime-package-lock "$runtime_package_lock"'
assert_contains '--runtime-snapshot "$runtime_snapshot"'
assert_contains '--runtime-replay "$runtime_replay"'
assert_contains 'cmp -s -- "$runtime_dir/$member" "$evidence_dir/$member"'
assert_contains 'Final upstream state and current-main check before staging upload'
assert_contains 'preflight_output="$RUNNER_TEMP/termux-mcp-server-v${EXPECTED_VERSION}-release-stage-${EXPECTED_COMMIT:0:12}.tar"'
assert_contains '[[ "$STAGED_ARTIFACT_DIGEST" =~ ^[0-9a-f]{64}$ ]]'
assert_contains 'test "$STAGED_ARTIFACT_DIGEST" = "$STAGED_TAR_SHA256"'
assert_contains "printf 'CI_RUN_ID=%s\\n' \"\$ci_run_id\" >>\"\$GITHUB_ENV\""
assert_contains "printf 'SECURITY_RUN_ID=%s\\n' \"\$security_run_id\" >>\"\$GITHUB_ENV\""
assert_contains "printf 'QUALIFICATION_RUN_ID=%s\\n' \"\$QUALIFICATION_RUN_ID\" >>\"\$GITHUB_ENV\""
assert_contains 'This workflow did not create a tag or GitHub Release.'

[[ "$(grep -Fc 'uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c' "$WORKFLOW")" -eq 6 ]] \
  || fail download_action_count_or_pin_changed
[[ "$(grep -Fc 'uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1' "$WORKFLOW")" -eq 2 ]] \
  || fail checkout_action_count_or_pin_changed
[[ "$(grep -Fc 'persist-credentials: false' "$WORKFLOW")" -eq 2 ]] \
  || fail checkout_credentials_must_remain_disabled
[[ "$(grep -Fc 'RUN_ATTEMPT: ${{ github.run_attempt }}' "$WORKFLOW")" -eq 2 ]] \
  || fail first_attempt_identity_not_checked_in_both_jobs
[[ "$(grep -Fc 'test "$RUN_ATTEMPT" = 1' "$WORKFLOW")" -eq 2 ]] \
  || fail workflow_rerun_guard_not_repeated
if grep -Fq 'ref: ${{ inputs.expected_commit }}' "$WORKFLOW"; then
  fail operator_input_must_not_select_checkout_ref
fi
[[ "$(grep -Fc 'digest-mismatch: error' "$WORKFLOW")" -eq 6 ]] \
  || fail download_digest_mismatch_must_fail
[[ "$(grep -Fc 'uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a' "$WORKFLOW")" -eq 1 ]] \
  || fail upload_action_count_or_pin_changed
[[ "$(grep -Fc "scripts/stage_release_assets.sh \\" "$WORKFLOW")" -eq 2 ]] \
  || fail assembler_must_run_before_and_after_approval
[[ "$(grep -Fc 'actions/workflows/$workflow_file/runs?branch=main&event=push&head_sha=' "$WORKFLOW")" -eq 4 ]] \
  || fail generic_latest_exact_head_check_count_changed
[[ "$(grep -Fc 'actions/workflows/android-cross-compile.yml/runs?branch=main&event=push&head_sha=' "$WORKFLOW")" -eq 1 ]] \
  || fail protected_android_latest_exact_head_check_count_changed
[[ "$(grep -Fc 'actions/workflows/automated-release-qualification.yml/runs?branch=main&event=workflow_run&head_sha=$EXPECTED_COMMIT&per_page=100' "$WORKFLOW")" -eq 3 ]] \
  || fail qualification_latest_run_check_count_changed
[[ "$(grep -Fc 'include "latest_workflow_run"' "$WORKFLOW")" -eq 9 ]] \
  || fail shared_latest_run_module_load_count_changed
[[ "$(grep -Fc 'complete_workflow_run_page' "$WORKFLOW")" -eq 9 ]] \
  || fail complete_latest_run_page_check_count_changed
[[ "$(grep -Fc '$latest.id == $run_id' "$WORKFLOW")" -eq 7 ]] \
  || fail latest_run_identity_check_count_changed
[[ "$(grep -Fc 'actions/runs/$ANDROID_RUN_ID/artifacts?per_page=100' "$WORKFLOW")" -eq 3 ]] \
  || fail artifact_inventory_check_count_changed
[[ "$(grep -Fc 'expired:.expired' "$WORKFLOW")" -eq 3 ]] \
  || fail artifact_identity_projection_must_bind_expiration_state
[[ "$(grep -Fc 'actions/runs/$QUALIFICATION_RUN_ID/artifacts?per_page=100' "$WORKFLOW")" -eq 2 ]] \
  || fail qualification_artifact_inventory_check_count_changed
[[ "$(grep -Fc 'actions/runs/$QUALIFICATION_RUN_ID"' "$WORKFLOW")" -eq 2 ]] \
  || fail qualification_run_requery_count_changed
assert_contains '"Android Cross Compile|android-cross-compile.yml|$ANDROID_RUN_ID"'
[[ "$(grep -Fc '(.inputs | keys | sort) == [' "$WORKFLOW")" -eq 2 ]] \
  || fail closed_dispatch_input_validation_not_repeated
[[ "$(grep -Fc 'expected_evidence_members="$(' "$WORKFLOW")" -eq 2 ]] \
  || fail exact_twelve_member_evidence_inventory_not_repeated
[[ "$(grep -Fc 'test "$actual_evidence_members" = "$expected_evidence_members"' "$WORKFLOW")" -eq 2 ]] \
  || fail exact_twelve_member_evidence_check_not_repeated
[[ "$(grep -Fc 'test "$(find "$evidence_dir" -mindepth 1 -maxdepth 1 -type f | wc -l)" = 12' "$WORKFLOW")" -eq 2 ]] \
  || fail exact_twelve_member_evidence_count_not_repeated
[[ "$(grep -Fc 'expected_runtime_members="$(' "$WORKFLOW")" -eq 2 ]] \
  || fail exact_runtime_source_inventory_not_repeated
[[ "$(grep -Fc 'test "$actual_runtime_members" = "$expected_runtime_members"' "$WORKFLOW")" -eq 2 ]] \
  || fail exact_runtime_source_check_not_repeated
[[ "$(grep -Fc 'cmp -s -- "$runtime_dir/$member" "$evidence_dir/$member"' "$WORKFLOW")" -eq 2 ]] \
  || fail runtime_source_qualification_join_not_repeated
[[ "$(grep -Fc 'mkdir -m 700 "$component_dir"' "$WORKFLOW")" -eq 2 ]] \
  || fail private_component_snapshot_not_repeated
[[ "$(grep -Fc 'install -m 600 -- "$evidence_dir/$member" "$component_dir/$member"' "$WORKFLOW")" -eq 2 ]] \
  || fail component_snapshot_copy_not_repeated
[[ "$(grep -Fc -- '--emulated-evidence-dir "$component_dir"' "$WORKFLOW")" -eq 2 ]] \
  || fail exact_six_member_component_directory_not_used_twice
if grep -Fq -- '--emulated-evidence-dir "$evidence_dir"' "$WORKFLOW"; then
  fail twelve_member_upstream_directory_passed_as_six_member_component_directory
fi
python3 - "$WORKFLOW" <<'PY'
import pathlib
import subprocess
import sys
import yaml

workflow_path = pathlib.Path(sys.argv[1])
text = workflow_path.read_text(encoding="utf-8")
document = yaml.safe_load(text)
for job_name, job in document["jobs"].items():
    if "continue-on-error" in job:
        raise SystemExit(f"{job_name} job may not ignore failure")
    if "if" in job:
        raise SystemExit(f"{job_name} job may not bypass success ordering")
    steps = job.get("steps")
    if not isinstance(steps, list) or not steps:
        raise SystemExit(f"{job_name} has no steps")
    names = []
    for index, step in enumerate(steps):
        if not isinstance(step, dict) or not isinstance(step.get("name"), str):
            raise SystemExit(f"{job_name} step {index} is empty or unnamed")
        if ("run" in step) == ("uses" in step):
            raise SystemExit(f"{job_name} step {index} must have exactly one action")
        if "continue-on-error" in step:
            raise SystemExit(
                f"{job_name} step {step['name']} may not ignore failure"
            )
        if "if" in step:
            raise SystemExit(
                f"{job_name} step {step['name']} may not bypass success ordering"
            )
        names.append(step["name"])
        if "run" in step:
            result = subprocess.run(
                ["bash", "-n"],
                input=step["run"],
                text=True,
                capture_output=True,
                check=False,
            )
            if result.returncode != 0:
                raise SystemExit(
                    f"embedded Bash syntax failed for {step['name']}: {result.stderr}"
                )
    if len(names) != len(set(names)):
        raise SystemExit(f"{job_name} contains duplicate step names")
    expected_steps = {
        "preflight": [
            "Checkout dispatch main candidate",
            "Validate dispatch and immutable source",
            "Resolve exact Android bundles and independent qualification",
            "Download exact seven Android bundles by ID",
            "Download exact independent qualification by ID",
            "Download exact qualified runtime snapshot by ID",
            "Verify companion runs and assemble throwaway stage",
        ],
        "stage": [
            "Require pre-created protected environment",
            "Checkout dispatch main candidate after approval",
            "Revalidate exact upstream identities after approval",
            "Re-download exact seven Android bundles by ID",
            "Re-download exact independent qualification by ID",
            "Re-download exact qualified runtime snapshot by ID",
            "Repeat evidence checks and assemble deterministic stage",
            "Final upstream state and current-main check before staging upload",
            "Upload staged tar without re-archiving",
            "Record staging identity",
        ],
    }
    if names != expected_steps[job_name]:
        raise SystemExit(f"{job_name} validation/download/order contract changed")

runtime_downloads = {
    "preflight": (
        "Download exact qualified runtime snapshot by ID",
        "${{ steps.upstream.outputs.runtime_artifact_id }}",
        "${{ inputs.android_run_id }}",
    ),
    "stage": (
        "Re-download exact qualified runtime snapshot by ID",
        "${{ needs.preflight.outputs.runtime_artifact_id }}",
        "${{ inputs.android_run_id }}",
    ),
}
for job_name, (step_name, artifact_id, run_id) in runtime_downloads.items():
    step = next(
        item for item in document["jobs"][job_name]["steps"]
        if item["name"] == step_name
    )
    expected_with = {
        "artifact-ids": artifact_id,
        "path": "upstream/runtime",
        "github-token": "${{ github.token }}",
        "repository": "${{ github.repository }}",
        "run-id": run_id,
        "digest-mismatch": "error",
    }
    if step.get("with") != expected_with:
        raise SystemExit(f"{step_name} exact immutable download contract changed")
preflight = text[text.index("  preflight:\n"):text.index("  stage:\n")]
stage = text[text.index("  stage:\n"):]
marker = "      - name: Repeat evidence checks and assemble deterministic stage\n"
start = stage.index(marker)
end = stage.index("\n      - name:", start + len(marker))
repeat_step = stage[start:end]
final_marker = "      - name: Final upstream state and current-main check before staging upload\n"
final_start = stage.index(final_marker)
final_end = stage.index("\n      - name:", final_start + len(final_marker))
final_step = stage[final_start:final_end]
for required in (
    "expected_android_names='[",
    '"termux-mcp-qualified-runtime-snapshot"',
    ".total_count == 9",
    "and (.artifacts | length) == 9",
    "and ([.artifacts[].name] | sort) == ($expected | sort)",
    "and ([.artifacts[].id] | unique | length) == 9",
    "and .expired == false",
    'select(.name == "termux-mcp-qualified-runtime-snapshot")',
    ".id == $runtime_artifact_id",
    "and .digest == $runtime_artifact_digest",
    "expired:.expired",
    'test "$final_android_artifact_set_sha256" = "$EXPECTED_ANDROID_ARTIFACT_SET_SHA256"',
):
    if required not in final_step:
        raise SystemExit(
            f"final Android artifact inventory is not fail-closed: {required}"
        )
upload_marker = "      - name: Upload staged tar without re-archiving\n"
upload_start = stage.index(upload_marker)
upload_end = stage.index("\n      - name:", upload_start + len(upload_marker))
upload_step = stage[upload_start:upload_end]
if "archive: false" not in upload_step or "path: ${{ env.STAGED_TAR }}" not in upload_step:
    raise SystemExit("raw staged-artifact upload contract changed")
if "\n          name:" in upload_step:
    raise SystemExit("raw upload must rely on its file basename; name input is ignored")
validation = '[[ "$run_id" =~ ^[1-9][0-9]*$ ]]'
for export in (
    "printf 'CI_RUN_ID=%s\\n' \"$ci_run_id\" >>\"$GITHUB_ENV\"",
    "printf 'SECURITY_RUN_ID=%s\\n' \"$security_run_id\" >>\"$GITHUB_ENV\"",
    "printf 'QUALIFICATION_RUN_ID=%s\\n' \"$QUALIFICATION_RUN_ID\" >>\"$GITHUB_ENV\"",
):
    if export in preflight:
        raise SystemExit("companion run ID export escaped into preflight job")
    if repeat_step.count(export) != 1:
        raise SystemExit("companion run ID export is not scoped to the protected repeat step")
    if repeat_step.index(validation) >= repeat_step.index(export):
        raise SystemExit("companion run ID is exported before validation")
PY
if grep -Eiq 'physical_bundle|physical-qualification|release-validator-v11|base64 --decode' "$WORKFLOW"; then
  fail automated_stage_must_not_accept_operator_physical_evidence
fi

for artifact in \
  termux-mcp-server-aarch64-linux-android-default \
  termux-mcp-server-aarch64-linux-android-mcp-runtime \
  termux-mcp-server-aarch64-linux-android-android-battery-status \
  termux-mcp-server-aarch64-linux-android-android-volume-status \
  termux-mcp-server-aarch64-linux-android-android-volume-control \
  termux-mcp-server-aarch64-linux-android-command-execution \
  termux-mcp-server-aarch64-linux-android-full-suite \
  termux-mcp-native-qualification-components \
  termux-mcp-qualified-runtime-snapshot \
  termux-mcp-native-qualification-evidence
do
  [[ "$(grep -Fc "$artifact" "$WORKFLOW")" -ge 2 ]] \
    || fail "artifact is not checked in both phases: $artifact"
done

if grep -Eiq -- '(^|[;&|[:space:]])cargo[[:space:]]+(build|check|clippy|fetch|metadata|run|test)|(^|[;&|[:space:]])(rustc|rustup)[[:space:]]|ANDROID_NDK|cross_compile|git[[:space:]]+tag|gh[[:space:]]+release|/releases([/?]|$)|contents:[[:space:]]*write|packages:|id-token:|deployments:' "$WORKFLOW"; then
  fail workflow_contains_build_or_publication_authority
fi
if grep -Eiq -- 'curl[^\n]*(--request|-X)[=[:space:]]*(POST|PUT|PATCH|DELETE)' "$WORKFLOW"; then
  fail mutating_rest_request_present
fi

if grep -Eq '^[[:space:]]+tags:' "$ANDROID_WORKFLOW"; then
  fail version_tag_still_triggers_android_rebuild
fi
[[ "$(grep -Fc 'retention-days: 30' "$ANDROID_WORKFLOW")" -eq 3 ]] \
  || fail android_qualification_retention_not_30_days
grep -Fq 'bash tests/release_staging_workflow_test.sh' "$CI_WORKFLOW" \
  || fail workflow_contract_not_run_by_ci

printf 'Release staging workflow contract passed\n'
