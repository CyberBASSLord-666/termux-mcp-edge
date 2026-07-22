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
assert_contains '.object.sha == $sha'
assert_contains '.run_attempt == 1'
assert_contains '.expired == false'
assert_contains '.total_count == 8'
assert_contains 'artifact-ids:'
assert_contains 'github-token: ${{ github.token }}'
assert_contains 'digest-mismatch: error'
assert_contains 'archive: false'
assert_contains 'retention-days: 30'
assert_contains 'staged_not_released'
assert_contains 'Release eligible: `false`'
assert_contains '(${#PHYSICAL_BUNDLE_GZIP_BASE64} <= 60000)'
assert_contains '[[ "$PHYSICAL_BUNDLE_GZIP_BASE64" =~ ^[A-Za-z0-9+/]+={0,2}$ ]]'
assert_contains '(${#PHYSICAL_BUNDLE_GZIP_BASE64} % 4 == 0)'
assert_contains 'PHYSICAL_BUNDLE_GZIP_BASE64="$(jq -er'
assert_contains '.inputs.physical_bundle_gzip_base64 as $value'
assert_contains 'and ($value | test("^[A-Za-z0-9+/]+={0,2}$"))'
assert_contains "printf '::add-mask::%s\\n' \"\$PHYSICAL_BUNDLE_GZIP_BASE64\""
assert_contains 'physical-qualification-v1.json'
assert_contains 'release-validator-v11.json'
assert_contains 'Final upstream state and current-main check before staging upload'
assert_contains 'preflight_output="$RUNNER_TEMP/termux-mcp-server-v${EXPECTED_VERSION}-release-stage-${EXPECTED_COMMIT:0:12}.tar"'
assert_contains '[[ "$STAGED_ARTIFACT_DIGEST" =~ ^[0-9a-f]{64}$ ]]'
assert_contains 'test "$STAGED_ARTIFACT_DIGEST" = "$STAGED_TAR_SHA256"'
assert_contains "printf 'CI_RUN_ID=%s\\n' \"\$ci_run_id\" >>\"\$GITHUB_ENV\""
assert_contains "printf 'SECURITY_RUN_ID=%s\\n' \"\$security_run_id\" >>\"\$GITHUB_ENV\""
assert_contains 'This workflow did not create a tag or GitHub Release.'

[[ "$(grep -Fc 'uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c' "$WORKFLOW")" -eq 2 ]] \
  || fail download_action_count_or_pin_changed
[[ "$(grep -Fc 'uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0' "$WORKFLOW")" -eq 2 ]] \
  || fail checkout_action_count_or_pin_changed
[[ "$(grep -Fc 'persist-credentials: false' "$WORKFLOW")" -eq 2 ]] \
  || fail checkout_credentials_must_remain_disabled
if grep -Fq 'ref: ${{ inputs.expected_commit }}' "$WORKFLOW"; then
  fail operator_input_must_not_select_checkout_ref
fi
[[ "$(grep -Fc 'digest-mismatch: error' "$WORKFLOW")" -eq 2 ]] \
  || fail download_digest_mismatch_must_fail
[[ "$(grep -Fc 'uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a' "$WORKFLOW")" -eq 1 ]] \
  || fail upload_action_count_or_pin_changed
[[ "$(grep -Fc "scripts/stage_release_assets.sh \\" "$WORKFLOW")" -eq 2 ]] \
  || fail assembler_must_run_before_and_after_approval
[[ "$(grep -Fc 'actions/workflows/android-cross-compile.yml/runs?branch=main&event=push&head_sha=' "$WORKFLOW")" -eq 2 ]] \
  || fail android_latest_exact_head_check_count_changed
[[ "$(grep -Fc 'actions/runs/$ANDROID_RUN_ID/artifacts?per_page=100' "$WORKFLOW")" -eq 3 ]] \
  || fail artifact_inventory_check_count_changed
[[ "$(grep -Fc 'actions/workflows/$workflow_file/runs?branch=main&event=push&head_sha=' "$WORKFLOW")" -eq 3 ]] \
  || fail companion_latest_exact_head_check_count_changed
assert_contains '"Android Cross Compile|android-cross-compile.yml|$ANDROID_RUN_ID"'
[[ "$(grep -Fc '(${#PHYSICAL_BUNDLE_GZIP_BASE64} <= 60000)' "$WORKFLOW")" -eq 4 ]] \
  || fail physical_input_bound_not_repeated
python3 - "$WORKFLOW" <<'PY'
import pathlib
import sys

text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
preflight = text[text.index("  preflight:\n"):text.index("  stage:\n")]
stage = text[text.index("  stage:\n"):]
marker = "      - name: Repeat evidence checks and assemble deterministic stage\n"
start = stage.index(marker)
end = stage.index("\n      - name:", start + len(marker))
repeat_step = stage[start:end]
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
):
    if export in preflight:
        raise SystemExit("companion run ID export escaped into preflight job")
    if repeat_step.count(export) != 1:
        raise SystemExit("companion run ID export is not scoped to the protected repeat step")
    if repeat_step.index(validation) >= repeat_step.index(export):
        raise SystemExit("companion run ID is exported before validation")
PY
[[ "$(grep -Fc '[[ "$PHYSICAL_BUNDLE_GZIP_BASE64" =~ ^[A-Za-z0-9+/]+={0,2}$ ]]' "$WORKFLOW")" -eq 4 ]] \
  || fail physical_input_alphabet_not_repeated
[[ "$(grep -Fc '.inputs.physical_bundle_gzip_base64 as $value' "$WORKFLOW")" -eq 4 ]] \
  || fail physical_input_event_validation_not_repeated
if grep -Fq 'PHYSICAL_BUNDLE_GZIP_BASE64: ${{ inputs.physical_bundle_gzip_base64 }}' "$WORKFLOW"; then
  fail physical_input_must_not_be_mapped_to_step_environment
fi

for artifact in \
  termux-mcp-server-aarch64-linux-android-default \
  termux-mcp-server-aarch64-linux-android-mcp-runtime \
  termux-mcp-server-aarch64-linux-android-android-battery-status \
  termux-mcp-server-aarch64-linux-android-android-volume-status \
  termux-mcp-server-aarch64-linux-android-android-volume-control \
  termux-mcp-server-aarch64-linux-android-command-execution \
  termux-mcp-server-aarch64-linux-android-full-suite \
  termux-mcp-emulated-evidence
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
[[ "$(grep -Fc 'retention-days: 30' "$ANDROID_WORKFLOW")" -eq 2 ]] \
  || fail android_qualification_retention_not_30_days
grep -Fq 'bash tests/release_staging_workflow_test.sh' "$CI_WORKFLOW" \
  || fail workflow_contract_not_run_by_ci

printf 'Release staging workflow contract passed\n'
