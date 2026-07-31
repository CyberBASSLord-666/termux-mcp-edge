#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$REPO_ROOT"

fail() {
  printf 'documentation contract failed: %s\n' "$1" >&2
  exit 1
}

mapfile -t markdown_files < <(git ls-files '*.md')
((${#markdown_files[@]} > 0)) || fail no_markdown_files

if grep -Fn 'current/bin/termux-mcp-server' "${markdown_files[@]}"; then
  fail obsolete_deployed_binary_path
fi

catalog=docs/CAPABILITIES.md
[[ -f "$catalog" ]] || fail capability_catalog_missing

python3 - "$catalog" <<'PY'
import pathlib
import re
import sys

catalog = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
source = pathlib.Path("src/mcp_transport.rs").read_text(encoding="utf-8")
all_source = "\n".join(
    path.read_text(encoding="utf-8") for path in sorted(pathlib.Path("src").rglob("*.rs"))
)

constants = dict(
    re.findall(r'(?:pub\s+)?const\s+([A-Z0-9_]+_TOOL):\s*&str\s*=\s*"([^"]+)";', all_source)
)
array_match = re.search(
    r'const\s+BASE_AVAILABLE_TOOLS:\s*\[&str;\s*(\d+)\]\s*=\s*\[(.*?)\];',
    source,
    re.DOTALL,
)
if array_match is None:
    raise SystemExit("BASE_AVAILABLE_TOOLS could not be parsed")

declared_count = int(array_match.group(1))
symbols = re.findall(r'\b[A-Z0-9_]+_TOOL\b', array_match.group(2))
if len(symbols) != declared_count:
    raise SystemExit("BASE_AVAILABLE_TOOLS declared and parsed counts differ")

optional_symbols = [
    "ANDROID_BATTERY_STATUS_TOOL",
    "ANDROID_VOLUME_STATUS_TOOL",
    "SET_ANDROID_VOLUME_TOOL",
    "RUN_COMMAND_PROFILE_TOOL",
]
for symbol in symbols + optional_symbols:
    tool = constants.get(symbol)
    if tool is None:
        raise SystemExit(f"tool constant could not be resolved: {symbol}")
    if f"`{tool}`" not in catalog:
        raise SystemExit(f"capability catalog omits source tool: {tool}")

if declared_count != 17:
    raise SystemExit("baseline tool count changed without documentation-contract review")
if "Up to 21" not in catalog and "exactly 21" not in catalog.lower():
    raise SystemExit("capability catalog omits the current maximum tool count")
PY

postures=(
  mcp-runtime
  android-battery-status
  android-volume-status
  android-volume-control
  command-execution
  full-suite
)
for posture in "${postures[@]}"; do
  grep -Fq -- "BUILD_FEATURES=$posture" CONTRIBUTING.md \
    || fail "contributing_posture_missing_$posture"
done

grep -Fq 'full-suite = ["mcp-runtime", "android-battery-status", "android-volume-status", "android-volume-control", "command-execution"]' Cargo.toml \
  || fail full_suite_feature_alias_invalid
grep -Fq 'cargo build --release --locked --features full-suite' README.md \
  || fail readme_full_suite_build_missing
grep -Fq 'cargo build --release --locked --all-features' README.md \
  || fail readme_raw_all_features_build_missing
grep -Fq 'must not substitute for `full-suite`' README.md \
  || fail readme_full_suite_all_features_distinction_missing
grep -Fiq 'exactly 17' docs/CAPABILITIES.md \
  || fail capability_catalog_full_suite_disabled_count_missing
grep -Fiq 'exactly 21' docs/CAPABILITIES.md \
  || fail capability_catalog_full_suite_enabled_count_missing
grep -Fq 'termux-mcp-server-aarch64-linux-android-full-suite' docs/ANDROID_ARTIFACTS.md \
  || fail android_full_suite_workflow_artifact_missing
grep -Fq 'termux-mcp-server-v0.7.0-aarch64-linux-android-full-suite' docs/ANDROID_ARTIFACTS.md \
  || fail android_full_suite_durable_asset_missing
grep -Fq 'official_termux_native_automated_v1' docs/RELEASE_GOVERNANCE.md \
  || fail release_governance_automated_qualification_missing
grep -Fq 'closed automated envelope' docs/V0.6.0_RELEASE_CANDIDATE.md \
  || fail release_candidate_automated_envelope_missing
grep -Fq 'schema/gate-v4' docs/V0.6.0_RELEASE_CANDIDATE.md \
  || fail release_candidate_aggregate_v4_missing
grep -Fq 'harness v11' docs/DEVICE_PRODUCTION_GATE.md \
  || fail device_harness_v11_missing
grep -Fq 'separately records its locked on-device native-build digest' docs/DEVICE_PRODUCTION_GATE.md \
  || fail device_harness_native_digest_boundary_missing
grep -Fq 'optional physical-certification tier' docs/DEVICE_PRODUCTION_GATE.md \
  || fail device_harness_optional_tier_missing
if grep -Eiq 'device[- ]harness[^.]*bound to (the )?(exact )?full-suite digest|harness[^.]*same full-suite digest' \
  docs/DEVICE_PRODUCTION_GATE.md docs/RELEASE_GOVERNANCE.md \
  docs/V0.6.0_RELEASE_CANDIDATE.md docs/EMULATED_RELEASE_GATE.md docs/OPERATIONS.md; then
  fail cross_toolchain_digest_equality_claim
fi
grep -Fq 'official_termux_native_automated_v1' docs/AUTOMATED_RELEASE_QUALIFICATION.md \
  || fail automated_qualification_class_missing
grep -Fq 'physicalDeviceObserved": false' docs/AUTOMATED_RELEASE_QUALIFICATION.md \
  || fail automated_qualification_physical_boundary_missing
grep -Fq 'androidFrameworkObserved": false' docs/AUTOMATED_RELEASE_QUALIFICATION.md \
  || fail automated_qualification_framework_boundary_missing
grep -Fq 'sustainedPhysicalSoak": false' docs/AUTOMATED_RELEASE_QUALIFICATION.md \
  || fail automated_qualification_soak_boundary_missing
grep -Fq 'physicalCertification": "not_run"' docs/AUTOMATED_RELEASE_QUALIFICATION.md \
  || fail automated_qualification_certification_boundary_missing
grep -Fq 'same exact qualified full-suite artifact and version' docs/AUTOMATED_RELEASE_QUALIFICATION.md \
  || fail automated_deployment_same_version_boundary_missing
grep -Fq 'they do not prove compatibility between' docs/AUTOMATED_RELEASE_QUALIFICATION.md \
  || fail automated_deployment_cross_version_nonclaim_missing
grep -Fq 'different application versions.' docs/AUTOMATED_RELEASE_QUALIFICATION.md \
  || fail automated_deployment_cross_version_nonclaim_missing
if grep -Fq 'on-device install/upgrade/rollback smoke procedure succeeds' docs/PRODUCTION_READINESS.md; then
  fail ordinary_release_physical_smoke_prerequisite_stale
fi
if grep -Fq 'Release validator v11 and device harness v11 must' docs/PRODUCTION_READINESS.md; then
  fail ordinary_release_device_harness_prerequisite_stale
fi
qualification_boundary='Automated release qualification proves the exact artifacts under the digest-pinned official Termux userland on native ARM64, including deterministic Android-provider simulation and isolated deployment recovery. It does not certify physical-device, OEM, battery-aging, thermal-soak, radio, Doze, or Android-framework behavior.'
for document in \
  README.md \
  CHANGELOG.md \
  docs/ANDROID_ARTIFACTS.md \
  docs/AUTOMATED_RELEASE_QUALIFICATION.md \
  docs/EMULATED_RELEASE_GATE.md \
  docs/PUBLIC_RELEASE.md \
  docs/RELEASE_GOVERNANCE.md \
  docs/SECURITY.md \
  docs/VALIDATION.md \
  docs/V0.6.0_RELEASE_CANDIDATE.md \
  docs/operator-validation.md
do
  grep -Fq "$qualification_boundary" "$document" \
    || fail "automated_qualification_boundary_missing_${document//\//_}"
done
grep -Fq 'The candidate was formally cancelled on 2026-07-31' docs/V0.6.0_RELEASE_CANDIDATE.md \
  || fail release_candidate_cancellation_missing
grep -Fq 'No `v0.6.0` tag or' docs/V0.6.0_RELEASE_CANDIDATE.md \
  || fail release_candidate_historical_no_tag_boundary_missing
grep -Fq 'GitHub Release ever existed' docs/V0.6.0_RELEASE_CANDIDATE.md \
  || fail release_candidate_historical_no_tag_boundary_missing
grep -Fq 'They cannot be' docs/V0.6.0_RELEASE_CANDIDATE.md \
  || fail release_candidate_noninheritance_missing
grep -Fq 'inherited by v0.7.0' docs/V0.6.0_RELEASE_CANDIDATE.md \
  || fail release_candidate_noninheritance_missing
grep -Fq 'publicationState: "staged_not_released"' docs/PUBLIC_RELEASE.md \
  || fail public_release_staged_not_released_boundary_missing
grep -Fq 'RELEASE_QUALIFICATION_PROTECTED=required-reviewer-main-only-v1' docs/PUBLIC_RELEASE.md \
  || fail protected_environment_guard_documentation_missing
grep -Fq 'organization and repository variable scopes must **not** define `RELEASE_QUALIFICATION_PROTECTED`' docs/PUBLIC_RELEASE.md \
  || fail protected_environment_broader_scope_exclusion_missing
grep -Fq 'retained for 30 days' docs/PUBLIC_RELEASE.md \
  || fail qualification_retention_window_missing
grep -Fq 'The staged Actions artifact is retained for 30 days' docs/PUBLIC_RELEASE.md \
  || fail staging_retention_window_missing
grep -Fq 'not confidential storage' docs/PUBLIC_RELEASE.md \
  || fail public_repository_artifact_confidentiality_boundary_missing
grep -Fq 'the three non-secret scalar staging inputs' docs/PUBLIC_RELEASE.md \
  || fail staging_scalar_input_confidentiality_boundary_missing
if grep -Fq 'both staging dispatch files' docs/PUBLIC_RELEASE.md; then
  fail stale_staging_dispatch_file_wording_present
fi
grep -Fq 'release-automated-qualification-schema-v1.json' docs/PUBLIC_RELEASE.md \
  || fail automated_qualification_schema_link_missing
grep -Fq 'release-staging-manifest-schema-v2.json' docs/PUBLIC_RELEASE.md \
  || fail release_staging_schema_link_missing
grep -Fq 'There is no evidence bundle to encode and no physical-evidence workflow input' docs/PUBLIC_RELEASE.md \
  || fail operator_evidence_dispatch_removed
grep -Fq 'cannot be submitted to the automated route' docs/PUBLIC_RELEASE.md \
  || fail physical_qualification_class_isolation_missing
grep -Fq 'first-attempt successful CI, Security, and Android push runs' docs/RELEASE_GOVERNANCE.md \
  || fail release_first_attempt_only_boundary_missing
grep -Fq 'Pre-create one **empty draft** GitHub Release' docs/RELEASE_GOVERNANCE.md \
  || fail empty_draft_before_attachment_missing
grep -Fq 'Obtain the disjoint `release-final` approval only after step 15 retains its closed record and reviewer-readable summary' docs/RELEASE_GOVERNANCE.md \
  || fail disjoint_final_publication_approval_missing
grep -Fq 'RELEASE_PRODUCTION_PROTECTED=asset-attachment-reviewer-main-only-v1' docs/PUBLIC_RELEASE.md \
  || fail release_production_environment_guard_missing
grep -Fq 'RELEASE_FINAL_PROTECTED=final-publication-reviewer-main-only-immutable-v1' docs/PUBLIC_RELEASE.md \
  || fail release_final_environment_guard_missing
grep -Fq 'RELEASE_FINAL_EXCLUSIVE_MUTATION_FREEZE=exclusive-release-main-policy-tag-writers-paused-v1' docs/PUBLIC_RELEASE.md \
  || fail release_final_mutation_freeze_guard_missing
grep -Fq "GitHub's Release update API has no compare-and-swap precondition" docs/PUBLIC_RELEASE.md \
  || fail publication_non_atomic_api_boundary_missing
grep -Fq 'The workflow'\''s repeated API reads are latest-observed checks, not an atomic repository-wide lock.' docs/PUBLIC_RELEASE.md \
  || fail publication_latest_observed_boundary_missing
grep -Fq '`RELEASE_PRODUCTION_POLICY_READ_TOKEN`' docs/PUBLIC_RELEASE.md \
  || fail release_production_policy_read_credential_missing
grep -Fq '`RELEASE_FINAL_POLICY_READ_TOKEN`' docs/PUBLIC_RELEASE.md \
  || fail release_final_policy_read_credential_missing
grep -Fq "limited to this repository's **Administration: read** permission" docs/PUBLIC_RELEASE.md \
  || fail publication_policy_credential_scope_missing
grep -Fq 'The two eligible-reviewer sets must be disjoint' docs/PUBLIC_RELEASE.md \
  || fail publication_disjoint_reviewers_missing
grep -Fq 'pre-created empty draft' docs/PUBLIC_RELEASE.md \
  || fail publication_empty_draft_boundary_missing
grep -Fq 'exact version title, a blank body' docs/PUBLIC_RELEASE.md \
  || fail publication_blank_draft_body_missing
grep -Fq 'bind the deterministic, provenance-derived release body' docs/PUBLIC_RELEASE.md \
  || fail publication_deterministic_body_binding_missing
grep -Fq 'contain exactly sixteen assets' docs/PUBLIC_RELEASE.md \
  || fail publication_sixteen_asset_allowlist_missing
grep -Fq 'the seven matching `<binary-name>.sha256` sidecars' docs/PUBLIC_RELEASE.md \
  || fail publication_seven_sidecars_missing
grep -Fq 'the unchanged raw `termux-mcp-server-v0.7.0-release-stage-<sha12>.tar`' docs/PUBLIC_RELEASE.md \
  || fail publication_raw_stage_asset_missing
grep -Fq 'receipt is verification state, not a seventeenth Release asset' docs/PUBLIC_RELEASE.md \
  || fail publication_receipt_asset_exclusion_missing
grep -Fq 'are not members of the sixteen-asset contract' docs/PUBLIC_RELEASE.md \
  || fail publication_source_archive_exclusion_missing
grep -Fq '**Independent byte verification.**' docs/PUBLIC_RELEASE.md \
  || fail publication_independent_byte_verification_missing
grep -Fq 'retains the closed JSON verification record for 30 days' docs/PUBLIC_RELEASE.md \
  || fail publication_verification_record_retention_missing
grep -Fq 'downloads that exact current-run verification artifact by server-assigned ID' docs/PUBLIC_RELEASE.md \
  || fail publication_same_run_verification_record_gate_missing
grep -Fq 'The Release body is bound before upload and contains only deterministic facts already available at that boundary' docs/PUBLIC_RELEASE.md \
  || fail publication_body_timing_boundary_missing
grep -Fq '## Operator worksheet (v0.7.0)' docs/PUBLIC_RELEASE.md \
  || fail publication_operator_worksheet_missing
for worksheet_field in \
  'expected_commit=<40-character current main SHA>' \
  'android_run_id=<successful first-attempt Android push run ID>' \
  'staged_artifact_id=<ID from the successful staging summary>' \
  'staged_artifact_sha256=<raw tar SHA-256 from the same summary>' \
  'expected_tag_object_sha=<annotated tag object SHA>' \
  'draft_release_id=<numeric ID of the one exact-tag empty draft>' \
  'asset_count=16' \
  'immutable=true'
do
  grep -Fq "$worksheet_field" docs/PUBLIC_RELEASE.md \
    || fail "publication_operator_worksheet_field_missing_$worksheet_field"
done
grep -Fq 'The `release-production` reviewer sees the successful read-only preflight and the pending protected job'\''s workflow definition' docs/PUBLIC_RELEASE.md \
  || fail publication_production_reviewer_context_missing
grep -Fq 'the attachment job'\''s repeated checks execute only after approval' docs/PUBLIC_RELEASE.md \
  || fail publication_production_postapproval_check_timing_missing
grep -Fq 'The independent verification record does not exist yet.' docs/PUBLIC_RELEASE.md \
  || fail publication_verification_record_timing_missing
grep -Fq 'The disjoint `release-final` reviewer acts only after the fresh read-only verification job has retained that record' docs/PUBLIC_RELEASE.md \
  || fail publication_final_reviewer_context_missing
if grep -Fq 'attachment job'\''s current checks' docs/PUBLIC_RELEASE.md; then
  fail publication_preapproval_attachment_check_overclaim_present
fi
for tag_safety_contract in \
  'set -euo pipefail' \
  'RELEASE_REMOTE_URL="https://github.com/$RELEASE_REPO.git"' \
  'RELEASE_COMMIT=PASTE_40_CHARACTER_EXPECTED_COMMIT_HERE' \
  'test "${#RELEASE_COMMIT}" -eq 40' \
  'expected_commit is not lowercase hexadecimal' \
  'git fetch --no-tags "$RELEASE_REMOTE_URL" main' \
  'git show-ref --verify --quiet "refs/tags/$RELEASE_TAG"' \
  'REMOTE_TAG_MATCHES="$(' \
  'git ls-remote --refs "$RELEASE_REMOTE_URL" "refs/tags/$RELEASE_TAG"' \
  'if test -n "$REMOTE_TAG_MATCHES"; then' \
  'git push "$RELEASE_REMOTE_URL"' \
  'test "$REMOTE_TAG_OBJECT_SHA" = "$TAG_OBJECT_SHA"'
do
  grep -Fq "$tag_safety_contract" docs/PUBLIC_RELEASE.md \
    || fail "publication_tag_safety_contract_missing_$tag_safety_contract"
done
if grep -Fq 'RELEASE_COMMIT=<copy' docs/PUBLIC_RELEASE.md; then
  fail unsafe_shell_redirection_placeholder_present
fi
if grep -Fq 'git ls-remote --exit-code' docs/PUBLIC_RELEASE.md; then
  fail remote_tag_transport_failure_can_look_absent
fi
grep -Fq 'Record `staging_run_id` from the numeric `/actions/runs/<id>` segment' docs/PUBLIC_RELEASE.md \
  || fail staging_run_id_capture_source_missing
grep -Fq '$matches[0].prerelease == false' docs/PUBLIC_RELEASE.md \
  || fail draft_capture_prerelease_filter_missing
grep -Fq 'gh api --paginate --slurp "repos/$RELEASE_REPO/releases?per_page=100" |' docs/PUBLIC_RELEASE.md \
  || fail draft_capture_pagination_flattening_missing
grep -Fq 'jq -er' docs/PUBLIC_RELEASE.md \
  || fail draft_capture_external_jq_missing
if grep -Eq 'gh api .*--slurp .*--jq|gh api .*--jq .*--slurp' docs/PUBLIC_RELEASE.md; then
  fail unsupported_gh_slurp_jq_combination_present
fi
grep -Fq '[ .[] | select(.tag_name == $release_tag) ] as $matches' docs/PUBLIC_RELEASE.md \
  || fail draft_capture_exact_tag_inventory_missing
grep -Fq 'if ($matches | length) != 1 then' docs/PUBLIC_RELEASE.md \
  || fail draft_capture_exact_tag_uniqueness_missing
grep -Fq 'error("expected exactly one Release for the tag")' docs/PUBLIC_RELEASE.md \
  || fail draft_capture_tag_count_error_missing
grep -Fq '(($matches[0].body // "") == "")' docs/PUBLIC_RELEASE.md \
  || fail draft_capture_blank_body_filter_missing
grep -Fq '(($matches[0].assets | length) == 0)' docs/PUBLIC_RELEASE.md \
  || fail draft_capture_zero_assets_filter_missing
grep -Fq 'error("exact-tag Release is not the required empty draft")' docs/PUBLIC_RELEASE.md \
  || fail draft_capture_object_contract_error_missing
grep -Fq 'draft Release ID is not one numeric value' docs/PUBLIC_RELEASE.md \
  || fail draft_capture_single_numeric_guard_missing
grep -Fq 'Because both protected jobs intentionally set `deployment: false`, they create no GitHub Deployment record' docs/PUBLIC_RELEASE.md \
  || fail publication_record_deployment_semantics_missing
grep -Fq "Use the linked run's environment-review UI" docs/PUBLIC_RELEASE.md \
  || fail publication_record_review_context_missing
grep -Fq 'every GitHub workflow rerun is rejected by the first-attempt guard' docs/PUBLIC_RELEASE.md \
  || fail publication_rerun_rejection_missing
grep -Fq 'must start a fresh reviewed dispatch' docs/PUBLIC_RELEASE.md \
  || fail publication_fresh_dispatch_recovery_missing
grep -Fq 'reasserts the already-verified `prerelease: false` state, and explicitly requests this Release as latest' docs/PUBLIC_RELEASE.md \
  || fail publication_patch_scope_documentation_missing
grep -Fq 'Server-assigned and post-upload facts belong to the separate workflow publication record' docs/RELEASE_GOVERNANCE.md \
  || fail governance_separate_publication_record_missing
grep -Fq 'The `release-production` reviewer sees the linked run'\''s successful read-only preflight and the pending protected job'\''s workflow definition' docs/RELEASE_GOVERNANCE.md \
  || fail governance_production_reviewer_context_missing
grep -Fq 'the attachment job'\''s repeated checks execute only after approval' docs/RELEASE_GOVERNANCE.md \
  || fail governance_production_postapproval_check_timing_missing
grep -Fq 'The independent-verification record does not exist until that approval and attachment succeed' docs/RELEASE_GOVERNANCE.md \
  || fail governance_verification_record_timing_missing
grep -Fq 'before the disjoint final environment review' docs/RELEASE_GOVERNANCE.md \
  || fail governance_final_reviewer_context_missing
if grep -Fq 'Every GitHub Release body must record' docs/RELEASE_GOVERNANCE.md; then
  fail governance_preupload_body_overclaim_present
fi
grep -Fq '`immutable: true`' docs/PUBLIC_RELEASE.md \
  || fail publication_immutable_true_proof_missing
grep -Fq 'public sixteen-asset re-download proof' docs/PUBLIC_RELEASE.md \
  || fail publication_public_redownload_proof_missing
grep -Fq 'workflow never auto-deletes or repairs a draft, asset, tag, or staging artifact' docs/PUBLIC_RELEASE.md \
  || fail publication_no_automatic_deletion_recovery_missing
grep -Fq 'Workflow bundles, stages, tags, and drafts are not installation sources' README.md \
  || fail readme_nonrelease_installation_boundary_missing
if grep -Eiq 'pre-existing immutable tag|verified immutable tag|independent final publication approval' \
  docs/PUBLIC_RELEASE.md docs/RELEASE_GOVERNANCE.md; then
  fail prepublication_immutability_or_unenforced_approval_claim
fi
python3 - <<'PY'
from pathlib import Path

text = Path("docs/PUBLIC_RELEASE.md").read_text(encoding="utf-8")
markers = [
    "**Public, non-confidential stage.**",
    "**Pre-created empty draft.**",
    "**Protected attachment.**",
    "**Independent byte verification.**",
    "**Separate final approval and mutation freeze.**",
    "**Immutable public proof.**",
]
positions = [text.index(marker) for marker in markers]
if positions != sorted(positions) or len(set(positions)) != len(positions):
    raise SystemExit("publication state-machine documentation order changed")
PY
grep -Fq 'Staging cannot tag or publish' README.md \
  || fail readme_staging_publication_boundary_missing
grep -Fq 'cargo clippy --locked --workspace --all-targets -- -D warnings' README.md \
  || fail readme_default_clippy_gate_missing
grep -Fq 'cargo test --locked --workspace --all-targets' README.md \
  || fail readme_default_test_gate_missing
grep -Fq 'At eight minutes, GNU `timeout` marks the posture failed and sends `TERM`; if it is still running 30 seconds later, `timeout` sends `KILL`.' docs/VALIDATION.md \
  || fail ci_test_posture_timeout_contract_missing
grep -Fq 'bash tests/release_staging_workflow_test.sh' README.md \
  || fail readme_release_staging_gate_missing
grep -Fq 'bash tests/release_publication_workflow_test.sh' README.md \
  || fail readme_release_publication_gate_missing
grep -Fq 'bash tests/prepare_release_publication_assets_test.sh' README.md \
  || fail readme_release_publication_preparer_gate_missing
grep -Fq 'bash tests/publish_release_assets_test.sh' README.md \
  || fail readme_release_publication_api_gate_missing
grep -Fq 'bash tests/termux_automated_deployment_gate_test.sh' README.md \
  || fail readme_automated_deployment_gate_missing
grep -Fq 'bash tests/package_automated_qualification_test.sh' README.md \
  || fail readme_automated_qualification_package_gate_missing
grep -Fq 'bash tests/runtime_snapshot_replay_test.sh' README.md \
  || fail readme_runtime_snapshot_replay_gate_missing
grep -Fq 'bash tests/runtime_snapshot_replay_test.sh' .github/workflows/ci.yml \
  || fail ci_runtime_snapshot_replay_gate_missing
grep -Fq 'bash tests/automated_release_qualification_workflow_test.sh' README.md \
  || fail readme_automated_qualification_workflow_gate_missing
grep -Fq 'bash tests/release_publication_workflow_test.sh' CONTRIBUTING.md \
  || fail contributing_release_publication_gate_missing
grep -Fq 'bash tests/prepare_release_publication_assets_test.sh' CONTRIBUTING.md \
  || fail contributing_release_publication_preparer_gate_missing
grep -Fq 'bash tests/publish_release_assets_test.sh' CONTRIBUTING.md \
  || fail contributing_release_publication_api_gate_missing
grep -Fq 'bash tests/termux_automated_deployment_gate_test.sh' CONTRIBUTING.md \
  || fail contributing_automated_deployment_gate_missing
grep -Fq 'bash tests/package_automated_qualification_test.sh' CONTRIBUTING.md \
  || fail contributing_automated_qualification_package_gate_missing
grep -Fq 'bash tests/runtime_snapshot_replay_test.sh' CONTRIBUTING.md \
  || fail contributing_runtime_snapshot_replay_gate_missing
grep -Fq 'bash tests/automated_release_qualification_workflow_test.sh' CONTRIBUTING.md \
  || fail contributing_automated_qualification_workflow_gate_missing
for physical_test in \
  package_shizuku_rish_test_artifact_test.sh \
  resolve_shizuku_rish_candidate_test.sh \
  rish_physical_identity_evidence_test.sh \
  shizuku_rish_physical_controller_test.sh \
  shizuku_rish_physical_workflow_test.sh \
  termux_rish_physical_gate_test.sh
do
  command="bash tests/$physical_test"
  grep -Fq "$command" README.md \
    || fail "readme_physical_gate_missing_${physical_test//[^A-Za-z0-9]/_}"
  grep -Fq "$command" CONTRIBUTING.md \
    || fail "contributing_physical_gate_missing_${physical_test//[^A-Za-z0-9]/_}"
  grep -Fq "$command" .github/workflows/ci.yml \
    || fail "ci_physical_gate_missing_${physical_test//[^A-Za-z0-9]/_}"
done
grep -Fq 'only when requesting the separate optional physical-certification tier' docs/VALIDATION.md \
  || fail validation_optional_physical_certification_boundary_missing
grep -Fq 'separate completed-run qualifier and protected staging review' docs/ANDROID_ARTIFACTS.md \
  || fail android_artifact_retention_purpose_stale
grep -Fq 'automated-native-deployment-scenarios-schema-v1.json' docs/AUTOMATED_RELEASE_QUALIFICATION.md \
  || fail automated_deployment_scenario_schema_documentation_missing
grep -Fq 'complete qualification boundary' README.md \
  || fail readme_qualification_publication_authority_conflated
if grep -Fq 'ordinary release authority' \
  README.md docs/AUTOMATED_RELEASE_QUALIFICATION.md docs/DEVICE_PRODUCTION_GATE.md; then
  fail qualification_class_mislabeled_as_publication_authority
fi
[[ "$(grep -Fc 'retention-days: 30' .github/workflows/android-cross-compile.yml)" -eq 3 ]] \
  || fail android_qualification_retention_contract_changed
grep -Fq 'exactly nine artifacts' docs/PUBLIC_RELEASE.md \
  || fail public_release_android_artifact_count_missing
grep -Fq 'twelve members' docs/PUBLIC_RELEASE.md \
  || fail public_release_qualifier_member_count_missing
grep -Fq 'exactly nine artifacts' docs/ANDROID_ARTIFACTS.md \
  || fail android_artifact_inventory_count_missing
grep -Fq 'twelve-member' docs/AUTOMATED_RELEASE_QUALIFICATION.md \
  || fail automated_qualification_member_count_missing
for runtime_name in \
  termux-qualified-runtime-image-v1.tar.gz \
  termux-runtime-package-lock-v1.json \
  termux-runtime-snapshot-v1.json \
  termux-runtime-snapshot-replay-v1.json
do
  grep -Fq "$runtime_name" scripts/prepare_release_publication_assets.sh \
    || fail "publication_runtime_inventory_missing_$runtime_name"
done
for claim in \
  'physicalDeviceObserved:false' \
  'androidFrameworkObserved:false' \
  'sustainedPhysicalSoak:false' \
  'physicalCertification:"not_run"' \
  'rebuildReproducibilityClaim:false'
do
  grep -Fq "$claim" README.md \
    || fail "readme_automated_negative_claim_missing_$claim"
  grep -Fq "$claim" docs/PRODUCTION_READINESS.md \
    || fail "production_readiness_negative_claim_missing_$claim"
done
for workflow in \
  .github/workflows/ci.yml \
  .github/workflows/security.yml \
  .github/workflows/android-cross-compile.yml
do
  for schema_pattern in \
    'docs/runtime-package-lock-schema-v*.json' \
    'docs/runtime-snapshot-schema-v*.json' \
    'docs/runtime-snapshot-replay-schema-v*.json'
  do
    grep -Fq "$schema_pattern" "$workflow" \
      || fail "runtime_schema_trigger_missing_${workflow}_${schema_pattern}"
  done
done
grep -Fq 'scripts/verify_runtime_snapshot.sh' .github/workflows/security.yml \
  || fail security_runtime_snapshot_verifier_trigger_missing
python3 - <<'PY'
import json
from pathlib import Path

qualification = json.loads(
    Path("docs/release-automated-qualification-schema-v1.json").read_text()
)
if "retainedRuntime" not in qualification["required"]:
    raise SystemExit("qualification retainedRuntime requirement missing")
if set(qualification["$defs"]["retainedRuntime"]["required"]) != {
    "runtimeImageId", "base", "archive", "packageLock", "snapshot", "replay",
    "installedPackages", "androidLinker", "verification", "claimBoundary",
    "rebuildReproducibilityClaim",
}:
    raise SystemExit("qualification retainedRuntime inventory changed")

stage = json.loads(
    Path("docs/release-staging-manifest-schema-v2.json").read_text()
)
runtime = stage["properties"]["evidence"]["properties"]["runtime"]
if runtime.get("additionalProperties") is not False:
    raise SystemExit("staging runtime record is not closed")
if set(runtime["required"]) != {"archive", "packageLock", "snapshot", "replay"}:
    raise SystemExit("staging runtime record inventory changed")
PY
if grep -Eq '^[[:space:]]+tags:' .github/workflows/android-cross-compile.yml; then
  fail tag_triggered_android_rebuild_present
fi
grep -Fq 'name: Stage Release Assets' .github/workflows/stage-release-assets.yml \
  || fail protected_release_staging_workflow_missing

publication_workflow=.github/workflows/publish-release.yml
if [[ -f "$publication_workflow" ]]; then
  grep -Fq 'name: Publish Immutable Release' "$publication_workflow" \
    || fail protected_release_publication_workflow_name_changed
  grep -Fq 'expected_tag_object_sha:' "$publication_workflow" \
    || fail protected_release_tag_object_input_missing
  grep -Fq 'staged_artifact_id:' "$publication_workflow" \
    || fail protected_release_stage_artifact_input_missing
  grep -Fq 'staged_artifact_sha256:' "$publication_workflow" \
    || fail protected_release_stage_digest_input_missing
  grep -Fq 'draft_release_id:' "$publication_workflow" \
    || fail protected_release_draft_id_input_missing
  grep -Fq 'release-production' "$publication_workflow" \
    || fail protected_release_production_environment_missing
  grep -Fq 'release-final' "$publication_workflow" \
    || fail protected_release_final_environment_missing
  grep -Fq 'asset-attachment-reviewer-main-only-v1' "$publication_workflow" \
    || fail protected_release_production_guard_missing
  grep -Fq 'final-publication-reviewer-main-only-immutable-v1' "$publication_workflow" \
    || fail protected_release_final_guard_missing
  grep -Fq 'secrets.RELEASE_PRODUCTION_POLICY_READ_TOKEN' "$publication_workflow" \
    || fail protected_release_production_policy_credential_missing
  grep -Fq 'secrets.RELEASE_FINAL_POLICY_READ_TOKEN' "$publication_workflow" \
    || fail protected_release_final_policy_credential_missing
fi

public_contract_docs=(
  README.md
  CONTRIBUTING.md
  SECURITY.md
  docs/CAPABILITIES.md
  docs/SECURITY.md
  docs/ANDROID_ARTIFACTS.md
  docs/PRODUCTION_READINESS.md
  docs/VALIDATION.md
  docs/OPERATIONS.md
  docs/TERMUX_DEPLOYMENT.md
  docs/RELEASE_GOVERNANCE.md
  docs/RELEASE_CANDIDATE_VALIDATION.md
  docs/DEVICE_PRODUCTION_GATE.md
  docs/EMULATED_RELEASE_GATE.md
  docs/V0.6.0_RELEASE_CANDIDATE.md
  docs/PUBLIC_RELEASE.md
  docs/MCP_RESTORATION_VALIDATION.md
  docs/MCP_RUNTIME_ROADMAP.md
  docs/TRANSPORT_THREAT_MODEL.md
  docs/operator-validation.md
  docs/EMBEDDING.md
  docs/command-profile-validation.md
  docs/command-execution-gate.md
  docs/capability-gates.md
  docs/SAFE_ROOT_BINARY_READS.md
  docs/SAFE_ROOT_BINARY_RANGES.md
  docs/SAFE_ROOT_PATH_DISCOVERY.md
  docs/SAFE_ROOT_TEXT_RANGES.md
  docs/SAFE_ROOT_FILE_WRITES.md
)
if grep -Eiq 'six (governed|supported|isolated) (android |compile-time |feature )?postures|all six Android|all six posture-specific|(release[- ]?)?validator[- ]v10|device[- ]harness[- ]v10' \
  "${public_contract_docs[@]}"; then
  fail stale_six_artifact_or_validator_contract
fi

grep -Fq '`trash_file`' SECURITY.md || fail root_security_trash_tool_missing
grep -Fq '`read_text_range`' SECURITY.md || fail root_security_text_range_missing
grep -Fq 'trash (`5`)' SECURITY.md || fail root_security_trash_family_missing
grep -Fq 'finite request-response SSE' SECURITY.md || fail root_security_sse_posture_missing
grep -Fq 'Copy, trash, and write results disclose neither' SECURITY.md \
  || fail root_security_result_privacy_scope_missing
grep -Fq 'Directory creation returns its normalized safe-rooted path' SECURITY.md \
  || fail root_security_create_result_scope_missing
grep -Fq '### `trash_file` request grant' docs/capability-gates.md \
  || fail trash_capability_gate_missing
python3 - <<'PY'
import pathlib
import re
import tomllib

manifest = tomllib.loads(pathlib.Path("Cargo.toml").read_text(encoding="utf-8"))
lock = tomllib.loads(pathlib.Path("Cargo.lock").read_text(encoding="utf-8"))
version = manifest["package"]["version"]
if re.fullmatch(r"\d+\.\d+\.\d+", version) is None:
    raise SystemExit("root package version is not release semver")

root = [
    package
    for package in lock["package"]
    if package["name"] == manifest["package"]["name"]
    and package.get("source") is None
]
if len(root) != 1 or root[0]["version"] != version:
    raise SystemExit("Cargo.lock root package version disagrees with Cargo.toml")

changelog = pathlib.Path("CHANGELOG.md").read_text(encoding="utf-8")
heading = f"## v{version}"
if len(re.findall(rf"(?m)^{re.escape(heading)}$", changelog)) != 1:
    raise SystemExit(f"CHANGELOG must contain exactly one {heading!r} heading")
first = re.search(r"(?m)^## .+$", changelog)
if first is None or first.group(0) != heading:
    raise SystemExit(f"current changelog section must be {heading!r}")
if "## v0.6.0" not in changelog or "Cancelled on 2026-07-31" not in changelog:
    raise SystemExit("cancelled v0.6.0 history is missing")
PY

if grep -Eiq -- '--private|create a new private repository' docs/GITHUB_IMPORT.md; then
  fail canonical_repository_visibility_stale
fi
grep -Fq 'https://github.com/CyberBASSLord-666/termux-mcp-edge.git' docs/GITHUB_IMPORT.md \
  || fail canonical_clone_url_missing
grep -Fq 'docs/**/*.md' .github/workflows/ci.yml || fail documentation_ci_path_filter_missing
grep -Fq 'pgrep -af "$PREFIX/bin/runsvdir"' README.md \
  || fail readme_service_supervisor_preflight_missing

python3 - "${markdown_files[@]}" <<'PY'
import pathlib
import re
import sys
import urllib.parse

link_pattern = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
resolving_cargo_command = re.compile(
    r"\bcargo\s+(?:build|check|clippy|fetch|metadata|run|test)\b"
)
locked_argument = re.compile(r"(?:^|\s)--locked(?:\s|$)")
shell_or_cargo_boundary = re.compile(r"\s--(?:\s|$)|\s#|&&|\|\||[;|]")
failures: list[str] = []


def command_uses_locked(segment: str, command: re.Match[str]) -> bool:
    arguments = segment[command.end():]
    boundary = shell_or_cargo_boundary.search(arguments)
    if boundary is not None:
        arguments = arguments[:boundary.start()]
    return locked_argument.search(arguments) is not None


for invalid_example in (
    "cargo test -- --locked",
    "cargo test && printf --locked",
    "cargo test # remember --locked",
):
    invalid_command = resolving_cargo_command.search(invalid_example)
    assert invalid_command is not None
    if command_uses_locked(invalid_example, invalid_command):
        raise SystemExit(f"documentation lock parser accepted invalid fixture: {invalid_example}")

for raw_name in sys.argv[1:]:
    document = pathlib.Path(raw_name)
    text = document.read_text(encoding="utf-8")
    in_fence = False
    for line_number, line in enumerate(text.splitlines(), start=1):
        if line.lstrip().startswith("```"):
            in_fence = not in_fence
            continue
        segments = [line] if in_fence else re.findall(r"`([^`]+)`", line)
        for segment in segments:
            for command in resolving_cargo_command.finditer(segment):
                if not command_uses_locked(segment, command):
                    failures.append(
                        f"{document}:{line_number}: public Cargo command does not use --locked before its argument boundary"
                    )
    for match in link_pattern.finditer(text):
        raw_target = match.group(1).strip()
        if not raw_target or raw_target.startswith(("#", "http://", "https://", "mailto:")):
            continue
        if raw_target.startswith("<") and raw_target.endswith(">"):
            raw_target = raw_target[1:-1]
        target_without_fragment = raw_target.split("#", 1)[0]
        target_without_fragment = urllib.parse.unquote(target_without_fragment)
        if not target_without_fragment:
            continue
        target = document.parent / target_without_fragment
        if not target.exists():
            line = text.count("\n", 0, match.start()) + 1
            failures.append(f"{document}:{line}: {raw_target}")

if failures:
    print("documentation contract violations:", file=sys.stderr)
    for failure in failures:
        print(f"  {failure}", file=sys.stderr)
    raise SystemExit(1)
PY

printf 'Documentation capability, deployment-path, posture, and link contracts passed\n'
