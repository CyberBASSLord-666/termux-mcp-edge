#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
WORKFLOW="$ROOT/.github/workflows/publish-release.yml"
PREPARER="$ROOT/scripts/prepare_release_publication_assets.sh"
PUBLISHER="$ROOT/scripts/publish_release_assets.sh"
CI_WORKFLOW="$ROOT/.github/workflows/ci.yml"

fail() {
  printf 'release publication workflow contract failed: %s\n' "$1" >&2
  exit 1
}

assert_contains() {
  local marker="$1"
  grep -Fq -- "$marker" "$WORKFLOW" || fail "missing marker: $marker"
}

[[ -f "$WORKFLOW" && ! -L "$WORKFLOW" ]] || fail workflow_missing_or_linked
[[ -f "$PREPARER" && ! -L "$PREPARER" ]] || fail preparer_missing_or_linked
[[ -f "$PUBLISHER" && ! -L "$PUBLISHER" ]] || fail publisher_missing_or_linked
[[ "$(grep -Ec '^[[:space:]]+workflow_dispatch:$' "$WORKFLOW")" -eq 1 ]] \
  || fail workflow_must_be_manual_only
if grep -Eq '^[[:space:]]+(push|pull_request|schedule):' "$WORKFLOW"; then
  fail automatic_publication_trigger_present
fi

assert_contains 'name: Publish Immutable Release'
assert_contains 'permissions: {}'
assert_contains 'group: production-release-publication'
assert_contains 'cancel-in-progress: false'
for input in \
  expected_commit \
  version \
  expected_tag_object_sha \
  staged_artifact_id \
  staged_artifact_sha256 \
  draft_release_id
do
  [[ "$(grep -Ec "^[[:space:]]{6}${input}:$" "$WORKFLOW")" -eq 1 ]] \
    || fail "dispatch input contract changed: $input"
done

for marker in \
  'refs/heads/main' \
  '[[ "$RUN_ATTEMPT" == 1 ]]' \
  'name: release-production' \
  'name: release-final' \
  'deployment: false' \
  'RELEASE_PRODUCTION_PROTECTED' \
  'asset-attachment-reviewer-main-only-v1' \
  'RELEASE_FINAL_PROTECTED' \
  'final-publication-reviewer-main-only-immutable-v1' \
  'RELEASE_PRODUCTION_POLICY_READ_TOKEN' \
  'RELEASE_FINAL_POLICY_READ_TOKEN' \
  'merge-multiple: true' \
  'digest-mismatch: error' \
  'prepare_release_publication_assets.sh' \
  'publish_release_assets.sh resolve-stage' \
  'publish_release_assets.sh preflight' \
  'publish_release_assets.sh attach' \
  'publish_release_assets.sh verify' \
  'publish_release_assets.sh publish' \
  'publish_release_assets.sh postverify' \
  'release-attachment-record-v1.json' \
  'release-draft-verification-record-v1.json' \
  '--verification-record' \
  'verification_record_artifact_id' \
  'actions/artifacts/$VERIFICATION_RECORD_ARTIFACT_ID' \
  'and .digest == $digest' \
  'for attempt in 1 2 3; do' \
  '408|425|429|500|502|503|504' \
  'sleep "$attempt"' \
  'Present independent record to final reviewer' \
  'Immutable public Release proof passed' \
  'immutable=true' \
  'public_asset_redownloads=16'
do
  assert_contains "$marker"
done

[[ "$(grep -Fc 'uses: actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0' "$WORKFLOW")" -eq 5 ]] \
  || fail checkout_action_count_or_pin_changed
[[ "$(grep -Fc 'persist-credentials: false' "$WORKFLOW")" -eq 5 ]] \
  || fail checkout_credentials_must_remain_disabled
if grep -Fq 'ref: ${{ inputs.' "$WORKFLOW"; then
  fail operator_input_must_not_select_checkout_ref
fi
[[ "$(grep -Fc 'uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c' "$WORKFLOW")" -eq 6 ]] \
  || fail download_action_count_or_pin_changed
[[ "$(grep -Fc 'digest-mismatch: error' "$WORKFLOW")" -eq 6 ]] \
  || fail staged_digest_mismatch_must_fail
[[ "$(grep -Fc 'uses: actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a' "$WORKFLOW")" -eq 2 ]] \
  || fail identity_record_upload_count_or_pin_changed
[[ "$(grep -Fc 'retention-days: 30' "$WORKFLOW")" -eq 2 ]] \
  || fail identity_record_retention_changed
if grep -Fq 'archive: false' "$WORKFLOW"; then
  fail identity_records_must_preserve_explicit_artifact_names
fi
[[ "$(grep -Fc 'publish_release_assets.sh resolve-stage' "$WORKFLOW")" -eq 5 ]] \
  || fail stage_resolution_must_repeat_in_every_job
[[ "$(grep -Fc 'prepare_release_publication_assets.sh' "$WORKFLOW")" -eq 5 ]] \
  || fail staged_byte_verification_must_repeat_in_every_job
[[ "$(grep -Fc '[[ "$RUN_ATTEMPT" == 1 ]]' "$WORKFLOW")" -eq 5 ]] \
  || fail reruns_must_be_rejected_in_every_job
[[ "$(grep -Fc 'actions: read' "$WORKFLOW")" -eq 5 ]] \
  || fail actions_permissions_changed
[[ "$(grep -Fc 'contents: read' "$WORKFLOW")" -eq 3 ]] \
  || fail read_only_job_count_changed
[[ "$(grep -Fc 'contents: write' "$WORKFLOW")" -eq 2 ]] \
  || fail write_job_count_changed
[[ "$(grep -Fc 'deployment: false' "$WORKFLOW")" -eq 2 ]] \
  || fail protected_environment_count_changed
[[ "$(grep -Fc 'GH_ADMIN_READ_TOKEN:' "$WORKFLOW")" -eq 2 ]] \
  || fail immutable_policy_token_scope_changed

python3 - "$WORKFLOW" <<'PY'
import pathlib
import sys

text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
lines = text.splitlines()
run_indent = None
for line in lines:
    indent = len(line) - len(line.lstrip(" "))
    if line.lstrip(" ") == "run: |":
        run_indent = indent
        continue
    if run_indent is not None and line.strip() and indent <= run_indent:
        run_indent = None
    if run_indent is not None and "${{ inputs." in line:
        raise SystemExit("operator input is interpolated directly into a shell block")

order = ["preflight", "attach-assets", "verify-draft", "publish", "postverify"]
sections = {}
for index, name in enumerate(order):
    marker = f"  {name}:\n"
    start = text.index(marker)
    if index + 1 < len(order):
        end = text.index(f"  {order[index + 1]}:\n", start + len(marker))
    else:
        end = len(text)
    sections[name] = text[start:end]

expected_needs = {
    "attach-assets": "needs: preflight",
    "verify-draft": "needs: attach-assets",
    "publish": "needs: verify-draft",
    "postverify": "needs: publish",
}
for job, marker in expected_needs.items():
    if marker not in sections[job]:
        raise SystemExit(f"publication DAG changed for {job}")

for job in ("preflight", "verify-draft", "postverify"):
    section = sections[job]
    if "contents: write" in section or "GH_ADMIN_READ_TOKEN:" in section or "environment:" in section:
        raise SystemExit(f"read-only job gained publication authority: {job}")

attach = sections["attach-assets"]
if "name: release-production" not in attach or "publish_release_assets.sh attach" not in attach:
    raise SystemExit("asset attachment escaped its protected job")
if "RELEASE_PRODUCTION_POLICY_READ_TOKEN" not in attach or "RELEASE_FINAL_POLICY_READ_TOKEN" in attach:
    raise SystemExit("asset attachment policy credential scope changed")
if "release-attachment-record-v1.json" not in attach or "actions/upload-artifact@" not in attach:
    raise SystemExit("attachment identity record is not retained in its protected job")

publish = sections["publish"]
if "name: release-final" not in publish or "publish_release_assets.sh publish" not in publish:
    raise SystemExit("publication escaped its protected job")
if "RELEASE_FINAL_POLICY_READ_TOKEN" not in publish or "RELEASE_PRODUCTION_POLICY_READ_TOKEN" in publish:
    raise SystemExit("final publication policy credential scope changed")
if "needs.verify-draft.outputs.verification_record_artifact_id" not in publish:
    raise SystemExit("final publication does not bind the independent record artifact ID")
if "needs.verify-draft.outputs.verification_record_file_sha256" not in publish:
    raise SystemExit("final publication does not bind the independent record file digest")
if "--verification-record" not in publish or "run-id: ${{ github.run_id }}" not in publish:
    raise SystemExit("final publication does not consume the exact same-run verification record")

verify = sections["verify-draft"]
if "release-draft-verification-record-v1.json" not in verify or "actions/upload-artifact@" not in verify:
    raise SystemExit("independent draft-verification record is not retained")
if "GITHUB_STEP_SUMMARY" not in verify or "Server SHA-256" not in verify:
    raise SystemExit("independent verification is not reviewer-readable")

for job, section in sections.items():
    expected = 1 if job in {"attach-assets", "publish"} else 0
    if section.count("contents: write") != expected:
        raise SystemExit(f"unexpected write authority in {job}")
PY

if grep -Eiq -- '(^|[;&|[:space:]])cargo[[:space:]]+(build|check|clippy|fetch|metadata|run|test)|(^|[;&|[:space:]])(rustc|rustup)[[:space:]]|ANDROID_NDK|cross_compile|stage_release_assets\.sh|git[[:space:]]+(tag|push)|gh[[:space:]]+release|packages:|id-token:|attestations:|deployments:' "$WORKFLOW"; then
  fail workflow_contains_build_tag_or_unneeded_authority
fi
if grep -Eiq -- 'POST.*/repos/[^/]+/[^/]+/releases(["[:space:]]|$)|/git/refs|--clobber|(^|[[:space:]])DELETE([[:space:]]|$)' "$WORKFLOW"; then
  fail workflow_contains_release_creation_tag_mutation_or_delete
fi

grep -Fq 'bash tests/prepare_release_publication_assets_test.sh' "$CI_WORKFLOW" \
  || fail preparer_test_not_run_by_ci
grep -Fq 'bash tests/publish_release_assets_test.sh' "$CI_WORKFLOW" \
  || fail publisher_test_not_run_by_ci
grep -Fq 'bash tests/release_publication_workflow_test.sh' "$CI_WORKFLOW" \
  || fail workflow_contract_not_run_by_ci

printf 'Immutable release publication workflow contract passed\n'
