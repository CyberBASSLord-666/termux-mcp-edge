#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
WORKFLOW="$ROOT/.github/workflows/automated-release-qualification.yml"
RUN_SELECTOR="$ROOT/scripts/latest_workflow_run.jq"
PACKAGER="$ROOT/scripts/package_automated_qualification.sh"
QUALIFICATION_SCHEMA="$ROOT/docs/release-automated-qualification-schema-v1.json"

fail() {
  printf 'automated release qualification workflow contract failed: %s\n' "$1" >&2
  exit 1
}

[[ -f "$WORKFLOW" && ! -L "$WORKFLOW" ]] || fail workflow_missing_or_linked
[[ -f "$RUN_SELECTOR" && ! -L "$RUN_SELECTOR" ]] \
  || fail latest_workflow_run_module_missing_or_linked
[[ -f "$PACKAGER" && ! -L "$PACKAGER" ]] || fail packager_missing_or_linked
[[ -f "$QUALIFICATION_SCHEMA" && ! -L "$QUALIFICATION_SCHEMA" ]] \
  || fail qualification_schema_missing_or_linked

python3 - "$WORKFLOW" "$RUN_SELECTOR" <<'PY'
import pathlib
import subprocess
import sys
import tempfile

try:
    import yaml
except ImportError as error:
    raise SystemExit(f"PyYAML is required to parse the workflow: {error}")

path = pathlib.Path(sys.argv[1])
run_selector_path = pathlib.Path(sys.argv[2])
try:
    text = path.read_text(encoding="utf-8")
    run_selector = run_selector_path.read_text(encoding="utf-8")
    document = yaml.safe_load(text)
except (OSError, UnicodeError, yaml.YAMLError) as error:
    raise SystemExit(f"workflow YAML did not parse: {error}")

if not isinstance(document, dict):
    raise SystemExit("workflow root is not a mapping")
if document.get("name") != "Automated Release Qualification":
    raise SystemExit("workflow name changed")

# PyYAML 1.1 treats the unquoted GitHub key `on` as boolean true.
trigger = document.get("on", document.get(True))
expected_trigger = {
    "workflow_run": {
        "workflows": ["Android Cross Compile"],
        "types": ["completed"],
    }
}
if trigger != expected_trigger:
    raise SystemExit(f"automatic trigger contract changed: {trigger!r}")
if document.get("permissions") != {"actions": "read", "contents": "read"}:
    raise SystemExit("workflow gained permissions beyond actions:read/contents:read")
workflow_defaults = document.get("defaults")
if (
    isinstance(workflow_defaults, dict)
    and isinstance(workflow_defaults.get("run"), dict)
    and "shell" in workflow_defaults["run"]
):
    raise SystemExit("workflow may not override the runner shell")

jobs = document.get("jobs")
if not isinstance(jobs, dict) or set(jobs) != {"qualify"}:
    raise SystemExit("workflow must contain exactly one qualification job")
job = jobs["qualify"]
if not isinstance(job, dict):
    raise SystemExit("qualification job is not a mapping")
if job.get("runs-on") != "ubuntu-24.04-arm":
    raise SystemExit("runtime replay must execute on the native ARM64 runner")
if job.get("timeout-minutes") != 45:
    raise SystemExit("qualification timeout changed")
if "permissions" in job:
    raise SystemExit("qualification job must inherit the exact read-only permissions")
if "continue-on-error" in job:
    raise SystemExit("qualification job must fail closed")
job_defaults = job.get("defaults")
if (
    isinstance(job_defaults, dict)
    and isinstance(job_defaults.get("run"), dict)
    and "shell" in job_defaults["run"]
):
    raise SystemExit("qualification job may not override the runner shell")
if job.get("env") != {
    "ANDROID_RUN_ID": "${{ github.event.workflow_run.id }}",
    "EXPECTED_COMMIT": "${{ github.event.workflow_run.head_sha }}",
    "QUALIFIER_RUN_ATTEMPT": "${{ github.run_attempt }}",
    "WORKFLOW_DEFINITION_REF": "${{ job.workflow_ref }}",
    "WORKFLOW_DEFINITION_REPOSITORY": "${{ job.workflow_repository }}",
    "WORKFLOW_DEFINITION_SHA": "${{ job.workflow_sha }}",
}:
    raise SystemExit("qualification job identity must come only from workflow_run")
condition = job.get("if")
if not isinstance(condition, str):
    raise SystemExit("qualification job has no closed workflow_run condition")
expected_condition = " && ".join(
    (
        "github.event.workflow_run.status == 'completed'",
        "github.event.workflow_run.conclusion == 'success'",
        "github.event.workflow_run.event == 'push'",
        "github.event.workflow_run.head_branch == 'main'",
        "github.event.workflow_run.run_attempt == 1",
        "github.run_attempt == '1'",
        "github.event.workflow_run.repository.full_name == github.repository",
        "github.event.workflow_run.head_repository.full_name == github.repository",
    )
)
if " ".join(condition.split()) != expected_condition:
    raise SystemExit("qualification job condition is not the exact closed predicate")

steps = job.get("steps")
if not isinstance(steps, list) or len(steps) != 6:
    raise SystemExit("qualification workflow step set changed")
for index, step in enumerate(steps):
    if not isinstance(step, dict):
        raise SystemExit(f"qualification step {index} is not a mapping")
    if "continue-on-error" in step:
        raise SystemExit(
            f"qualification step {step.get('name', index)} may not ignore failure"
        )
    if "if" in step:
        raise SystemExit(
            f"qualification step {step.get('name', index)} may not bypass success ordering"
        )
    if "shell" in step:
        raise SystemExit(
            f"qualification step {step.get('name', index)} may not override the runner shell"
        )
expected_step_names = [
    "Checkout exact completed Android source",
    "Verify source run and discover exact upstream artifacts",
    "Download exact Android artifacts by immutable ID",
    "Resolve companions and package exact automated qualification",
    "Final API requery of all runs and Android artifacts",
    "Upload immutable automated qualification evidence",
]
actual_step_names = [step.get("name") for step in steps]
if actual_step_names != expected_step_names:
    raise SystemExit("qualification workflow proof/upload step ordering changed")
by_name = {step["name"]: step for step in steps}

checkout = by_name["Checkout exact completed Android source"]
if checkout.get("uses") != "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1":
    raise SystemExit("checkout action pin changed")
checkout_with = checkout.get("with")
if checkout_with != {
    "repository": "${{ job.workflow_repository }}",
    "ref": "${{ job.workflow_sha }}",
    "fetch-depth": 1,
    "persist-credentials": False,
}:
    raise SystemExit("checkout must use only the immutable workflow-definition identity")

download = by_name["Download exact Android artifacts by immutable ID"]
if download.get("uses") != "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c":
    raise SystemExit("download action pin changed")
download_with = download.get("with")
if not isinstance(download_with, dict):
    raise SystemExit("download action inputs missing")
expected_download = {
    "artifact-ids": "${{ steps.upstream.outputs.artifact_ids }}",
    "path": "upstream",
    "github-token": "${{ github.token }}",
    "repository": "${{ github.repository }}",
    "run-id": "${{ github.event.workflow_run.id }}",
    "merge-multiple": False,
    "digest-mismatch": "error",
}
if download_with != expected_download:
    raise SystemExit("artifact download is not exact-ID/exact-run/digest-enforced")

upload = by_name["Upload immutable automated qualification evidence"]
if upload.get("uses") != "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a":
    raise SystemExit("upload action pin changed")
upload_with = upload.get("with")
if not isinstance(upload_with, dict):
    raise SystemExit("upload action inputs missing")
for key, expected in (
    ("name", "termux-mcp-native-qualification-evidence"),
    ("if-no-files-found", "error"),
    ("retention-days", 30),
    ("include-hidden-files", True),
    ("overwrite", False),
    ("compression-level", 0),
):
    if upload_with.get(key) != expected:
        raise SystemExit(f"immutable evidence upload option changed: {key}")
upload_paths = upload_with.get("path")
if not isinstance(upload_paths, str):
    raise SystemExit("upload path must enumerate the exact evidence files")
actual_members = {
    line.strip().rsplit("/", 1)[-1]
    for line in upload_paths.splitlines()
    if line.strip()
}
expected_members = {
    "automated-native-deployment-v1.json",
    "automated-qualification-v1.json",
    "termux-battery-emulated-evidence.json",
    "termux-command-emulated-evidence.json",
    "termux-native-aggregate-evidence-v4.json",
    "termux-observation-requirement-v3.json",
    "termux-qualified-runtime-image-v1.tar.gz",
    "termux-runtime-package-lock-v1.json",
    "termux-runtime-snapshot-replay-v1.json",
    "termux-runtime-snapshot-v1.json",
    "termux-volume-control-emulated-evidence.json",
    "termux-volume-emulated-evidence.json",
}
if actual_members != expected_members or len(upload_paths.splitlines()) != 12:
    raise SystemExit("uploaded qualification snapshot is not the exact twelve-file set")
if any(
    not line.strip().startswith("${{ steps.assemble.outputs.snapshot }}/")
    for line in upload_paths.splitlines()
    if line.strip()
):
    raise SystemExit("upload consumes a path outside the verified private snapshot")

allowed_actions = {
    "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
    "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c",
    "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
}
used_actions = {
    step["uses"]
    for step in steps
    if isinstance(step, dict) and isinstance(step.get("uses"), str)
}
if used_actions != allowed_actions:
    raise SystemExit("qualification workflow action set changed")

# Parse every embedded shell program with Bash as well as parsing the YAML.
for index, step in enumerate(steps):
    script = step.get("run") if isinstance(step, dict) else None
    if script is None:
        continue
    if not isinstance(script, str):
        raise SystemExit(f"step {index} run body is not a string")
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", suffix=".sh", delete=True
    ) as temporary:
        temporary.write(script)
        temporary.flush()
        result = subprocess.run(
            ["bash", "-n", temporary.name],
            capture_output=True,
            text=True,
            check=False,
        )
    if result.returncode != 0:
        raise SystemExit(
            f"embedded Bash syntax failed for {step.get('name')}: {result.stderr}"
        )

source = by_name["Verify source run and discover exact upstream artifacts"]["run"]
assemble = by_name["Resolve companions and package exact automated qualification"]["run"]
final = by_name["Final API requery of all runs and Android artifacts"]["run"]

for marker in (
    '.workflow_run.path == ".github/workflows/android-cross-compile.yml"',
    ".workflow_run.repository.full_name == $repository",
    ".workflow_run.head_repository.full_name == $repository",
    ".workflow_run.run_attempt == 1",
    ".workflow_run.status == \"completed\"",
    ".workflow_run.conclusion == \"success\"",
    'test "$QUALIFIER_RUN_ATTEMPT" = 1',
    'test "$WORKFLOW_DEFINITION_SHA" = "$EXPECTED_COMMIT"',
    '"$GITHUB_REPOSITORY/.github/workflows/automated-release-qualification.yml@refs/heads/main"',
    "actions/workflows/android-cross-compile.yml/runs?branch=main&event=push&head_sha=",
    'include "latest_workflow_run"',
    "complete_workflow_run_page",
    "$latest.id == $run_id",
    'api_get "$api_root/actions/runs/$ANDROID_RUN_ID/artifacts?per_page=100"',
    ".total_count == 9",
    "and ([.artifacts[].id] | unique | length) == 9",
    'and (.digest | test("^sha256:[0-9a-f]{64}$"))',
    "and .workflow_run.id == $run_id",
):
    if marker not in source:
        raise SystemExit(f"source run/artifact verification missing: {marker}")

for marker in (
    "termux-mcp-native-qualification-components",
    "termux-mcp-qualified-runtime-snapshot",
    "termux-mcp-server-aarch64-linux-android-default",
    "termux-mcp-server-aarch64-linux-android-mcp-runtime",
    "termux-mcp-server-aarch64-linux-android-android-battery-status",
    "termux-mcp-server-aarch64-linux-android-android-volume-status",
    "termux-mcp-server-aarch64-linux-android-android-volume-control",
    "termux-mcp-server-aarch64-linux-android-command-execution",
    "termux-mcp-server-aarch64-linux-android-full-suite",
):
    if marker not in source or marker not in assemble or marker not in final:
        raise SystemExit(f"artifact identity not preserved through all phases: {marker}")

for marker in (
    "verify_companion_run CI ci.yml",
    "verify_companion_run Security security.yml",
    "actions/workflows/$workflow_file/runs?branch=main&event=push&head_sha=",
    'include "latest_workflow_run"',
    "complete_workflow_run_page",
    "$latest.id == $run_id",
    ".path == $path",
    ".repository.full_name == $repository",
    ".head_repository.full_name == $repository",
    ".run_attempt == 1",
    "bash scripts/package_automated_qualification.sh",
    '--policy "$GITHUB_WORKSPACE/docs/release-qualification-policy-v1.json"',
    '--scenario-set "$GITHUB_WORKSPACE/docs/automated-native-deployment-scenarios-v1.json"',
    '--aggregate-evidence "$snapshot/termux-native-aggregate-evidence-v4.json"',
    '--deployment-evidence "$snapshot/automated-native-deployment-v1.json"',
    '--classifier-evidence "$snapshot/termux-observation-requirement-v3.json"',
    '--battery-evidence "$snapshot/termux-battery-emulated-evidence.json"',
    '--volume-evidence "$snapshot/termux-volume-emulated-evidence.json"',
    '--volume-control-evidence "$snapshot/termux-volume-control-emulated-evidence.json"',
    '--command-evidence "$snapshot/termux-command-emulated-evidence.json"',
    "bash scripts/verify_runtime_snapshot.sh",
    '--archive "$runtime_dir/termux-qualified-runtime-image-v1.tar.gz"',
    '--package-lock "$runtime_dir/termux-runtime-package-lock-v1.json"',
    '--snapshot-manifest "$runtime_dir/termux-runtime-snapshot-v1.json"',
    '--output "$snapshot/termux-runtime-snapshot-replay-v1.json"',
    '--runtime-archive "$snapshot/termux-qualified-runtime-image-v1.tar.gz"',
    '--runtime-package-lock "$snapshot/termux-runtime-package-lock-v1.json"',
    '--runtime-snapshot "$snapshot/termux-runtime-snapshot-v1.json"',
    '--runtime-replay "$snapshot/termux-runtime-snapshot-replay-v1.json"',
    '--output "$snapshot/automated-qualification-v1.json"',
    'test "$actual_snapshot_names" = "$expected_snapshot_names"',
    "official_termux_native_automated_v1",
    "physicalDeviceObserved: false",
    "androidFrameworkObserved: false",
    "sustainedPhysicalSoak: false",
    'physicalCertification: "not_run"',
):
    if marker not in assemble:
        raise SystemExit(f"canonical qualification packaging contract missing: {marker}")

permission_markers = (
    'expected_binary_artifact_names="$(',
    'expected_bundle_names="$(',
    'test ! -L "$artifact_dir"',
    'test "$actual_bundle_names" = "$expected_bundle_names"',
    'test ! -L "$artifact_dir/$metadata"',
    'test ! -x "$artifact_dir/$metadata"',
    'test ! -L "$binary"',
    'chmod 755 -- "$binary"',
    'test "$(stat -c %a "$binary")" = 755',
)
permission_positions = []
for marker in permission_markers:
    if assemble.count(marker) != 1:
        raise SystemExit(f"downloaded-binary mode normalization changed: {marker}")
    permission_positions.append(assemble.index(marker))
if permission_positions != sorted(permission_positions):
    raise SystemExit("binary inventory and mode normalization ordering changed")
if permission_positions[-1] >= assemble.index(
    "bash scripts/package_automated_qualification.sh"
):
    raise SystemExit("qualification packaging runs before binary mode normalization")

for marker in (
    'verify_run_unchanged "Android Cross Compile" android-cross-compile.yml',
    "verify_run_unchanged CI ci.yml",
    "verify_run_unchanged Security security.yml",
    'api_get "$api_root/actions/runs/$android_run_id/artifacts?per_page=100"',
    "android.artifacts.identity.final.json",
    'cmp -s \\\n  "$provenance_root/android.artifacts.identity.json"',
    'sha256sum --check "$provenance_root/evidence-snapshot.sha256"',
    'test "$(find "$SNAPSHOT" -mindepth 1 -maxdepth 1 -type f | wc -l)" = 12',
    'chmod 400 "$member"',
    'chmod 500 "$SNAPSHOT"',
    'api_get "$api_root/git/ref/heads/main"',
    '.ref == "refs/heads/main"',
    '.object.type == "commit"',
    ".object.sha == $sha",
    'include "latest_workflow_run"',
    "complete_workflow_run_page",
    "$latest.id == $run_id",
):
    if marker not in final:
        raise SystemExit(f"final immutable-upstream requery missing: {marker}")

main_query = final.index('api_get "$api_root/git/ref/heads/main"')
if main_query < final.rindex(
    'sha256sum --check "$provenance_root/evidence-snapshot.sha256"'
):
    raise SystemExit("current-main proof must follow all run/artifact/byte checks")
if not final.rstrip().endswith(
    "' \"$provenance_root/main.ref.final.json\" >/dev/null"
):
    raise SystemExit("current-main equality must be the final shell operation before upload")
if text.count('include "latest_workflow_run"') != 3:
    raise SystemExit("shared latest-run module must be loaded in all three phases")
if text.count("complete_workflow_run_page") != 3:
    raise SystemExit("complete latest-run pages must be required in all three phases")
if text.count("$latest.id == $run_id") != 3:
    raise SystemExit("latest exact-head run identity must be checked in all three phases")

# REST list order is not contractual. The newest failed run must win in either
# old-first or new-first input order. Creation/start ordering disagreement,
# including an older rerun, must fail closed.
latest_filter = (
    'include "latest_workflow_run"; '
    'complete_workflow_run_page '
    '| [.[] | select(.name == "CI")] '
    '| latest_workflow_run '
    '| .id == 2 and .conclusion == "failure"'
)
old = {
    "id": 1,
    "name": "CI",
    "created_at": "2026-07-23T00:00:00Z",
    "run_started_at": "2026-07-23T00:00:01Z",
    "conclusion": "success",
    "run_attempt": 1,
}
new = {
    "id": 2,
    "name": "CI",
    "created_at": "2026-07-23T00:01:00Z",
    "run_started_at": "2026-07-23T00:01:01Z",
    "conclusion": "failure",
    "run_attempt": 1,
}
import json
for runs in ([old, new], [new, old]):
    result = subprocess.run(
        ["jq", "-L", str(run_selector_path.parent), "-e", latest_filter],
        input=json.dumps({"total_count": len(runs), "workflow_runs": runs}),
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode != 0:
        raise SystemExit("latest-run sort depends on REST response order")

validation_filter = (
    'include "latest_workflow_run"; '
    'complete_workflow_run_page | latest_workflow_run_or_null'
)
rerun = dict(
    old,
    run_started_at=new["run_started_at"],
    run_attempt=2,
)
result = subprocess.run(
    ["jq", "-L", str(run_selector_path.parent), "-e", validation_filter],
    input=json.dumps({"total_count": 2, "workflow_runs": [new, rerun]}),
    text=True,
    capture_output=True,
    check=False,
)
if result.returncode == 0:
    raise SystemExit("same-second older rerun did not make run ordering fail closed")

crossed = dict(
    old,
    run_started_at="2026-07-23T00:02:00Z",
)
result = subprocess.run(
    ["jq", "-L", str(run_selector_path.parent), "-e", validation_filter],
    input=json.dumps({"total_count": 2, "workflow_runs": [crossed, new]}),
    text=True,
    capture_output=True,
    check=False,
)
if result.returncode == 0:
    raise SystemExit("crossed creation/start ordering did not fail closed")

invalid_pages = (
    {"total_count": 2, "workflow_runs": [old]},
    {"total_count": 101, "workflow_runs": [old]},
    {"total_count": 2, "workflow_runs": [old, old]},
    {
        "total_count": 1,
        "workflow_runs": [{**old, "created_at": "not-a-timestamp"}],
    },
)
for payload in invalid_pages:
    result = subprocess.run(
        ["jq", "-L", str(run_selector_path.parent), "-e", validation_filter],
        input=json.dumps(payload),
        text=True,
        capture_output=True,
        check=False,
    )
    if result.returncode == 0:
        raise SystemExit(f"invalid latest-run page was accepted: {payload!r}")
PY

if grep -Eq '^[[:space:]]+(workflow_dispatch|push|pull_request|schedule):' "$WORKFLOW"; then
  fail non_workflow_run_trigger_present
fi
if grep -Fq '${{ inputs.' "$WORKFLOW" || grep -Fq 'github.event.inputs' "$WORKFLOW"; then
  fail operator_input_surface_present
fi
if grep -Fq 'ref: ${{ github.sha }}' "$WORKFLOW" \
  || grep -Fq 'ref: ${{ github.ref }}' "$WORKFLOW" \
  || grep -Fq 'ref: ${{ github.event.workflow_run.head_branch }}' "$WORKFLOW" \
  || grep -Fq 'ref: ${{ github.event.workflow_run.head_sha }}' "$WORKFLOW"; then
  fail checkout_ref_is_not_immutable_workflow_definition
fi
if grep -Fq 'EXPECTED_COMMIT: ${{ github.sha }}' "$WORKFLOW" \
  || grep -Fq 'EXPECTED_COMMIT: ${{ github.workflow_sha }}' "$WORKFLOW" \
  || grep -Fq 'EXPECTED_COMMIT: ${{ job.workflow_sha }}' "$WORKFLOW"; then
  fail source_identity_must_come_only_from_triggering_android_head
fi
[[ "$(grep -Fc 'repository: ${{ job.workflow_repository }}' "$WORKFLOW")" -eq 1 ]] \
  || fail immutable_checkout_repository_count_changed
[[ "$(grep -Fc 'ref: ${{ job.workflow_sha }}' "$WORKFLOW")" -eq 1 ]] \
  || fail exact_checkout_ref_count_changed
[[ "$(grep -Fc "github.run_attempt == '1'" "$WORKFLOW")" -eq 1 ]] \
  || fail qualifier_own_first_attempt_job_guard_changed
[[ "$(grep -Fc 'test "$QUALIFIER_RUN_ATTEMPT" = 1' "$WORKFLOW")" -eq 1 ]] \
  || fail qualifier_own_first_attempt_shell_guard_changed
[[ "$(grep -Fc 'test "$WORKFLOW_DEFINITION_SHA" = "$EXPECTED_COMMIT"' "$WORKFLOW")" -eq 1 ]] \
  || fail workflow_definition_sha_binding_changed
[[ "$(grep -Fc 'test "$WORKFLOW_DEFINITION_REPOSITORY" = "$GITHUB_REPOSITORY"' "$WORKFLOW")" -eq 1 ]] \
  || fail workflow_definition_repository_binding_changed
[[ "$(grep -Fc 'automated-release-qualification.yml@refs/heads/main' "$WORKFLOW")" -eq 1 ]] \
  || fail workflow_definition_ref_binding_changed
[[ "$(grep -Fc 'api_get "$api_root/git/ref/heads/main"' "$WORKFLOW")" -eq 1 ]] \
  || fail final_current_main_query_changed
[[ "$(grep -Fc 'persist-credentials: false' "$WORKFLOW")" -eq 1 ]] \
  || fail checkout_credentials_must_remain_disabled
[[ "$(grep -Fc 'set +x' "$WORKFLOW")" -eq 3 ]] \
  || fail api_and_packaging_steps_must_disable_tracing
[[ "$(grep -Fc 'actions/runs/$run_id' "$WORKFLOW")" -ge 2 ]] \
  || fail companion_runs_not_queried_before_and_after_packaging
[[ "$(grep -Fc 'actions/runs/$ANDROID_RUN_ID/artifacts?per_page=100' "$WORKFLOW")" -eq 1 ]] \
  || fail initial_android_artifact_query_changed
[[ "$(grep -Fc 'actions/runs/$android_run_id/artifacts?per_page=100' "$WORKFLOW")" -eq 1 ]] \
  || fail final_android_artifact_query_changed

for member in \
  automated-native-deployment-v1.json \
  automated-qualification-v1.json \
  termux-battery-emulated-evidence.json \
  termux-command-emulated-evidence.json \
  termux-native-aggregate-evidence-v4.json \
  termux-observation-requirement-v3.json \
  termux-qualified-runtime-image-v1.tar.gz \
  termux-runtime-package-lock-v1.json \
  termux-runtime-snapshot-replay-v1.json \
  termux-runtime-snapshot-v1.json \
  termux-volume-control-emulated-evidence.json \
  termux-volume-emulated-evidence.json
do
  [[ "$(grep -Fc "$member" "$WORKFLOW")" -ge 3 ]] \
    || fail "twelve-file inventory is not closed through packaging/upload: $member"
done

if grep -Eiq -- 'contents:[[:space:]]*write|actions:[[:space:]]*write|security-events:|packages:|id-token:|attestations:|deployments:' "$WORKFLOW"; then
  fail workflow_contains_write_or_unneeded_authority
fi
if grep -Eiq -- 'curl[^\n]*(--request|-X)[=[:space:]]*(POST|PUT|PATCH|DELETE)|git[[:space:]]+(push|tag)|gh[[:space:]]+(api|release)|/releases([/?]|$)' "$WORKFLOW"; then
  fail mutating_api_or_repository_operation_present
fi
if grep -Eiq -- 'physical[_-](bundle|qualification)|base64[[:space:]]+(-d|--decode)|time[_-]?dilation|equivalent[_-]?minutes' "$WORKFLOW"; then
  fail operator_or_physical_evidence_substitution_surface_present
fi
if grep -Eiq -- '(^|[;&|[:space:]])cargo[[:space:]]+(build|check|clippy|fetch|run|test)|(^|[;&|[:space:]])(rustc|rustup)[[:space:]]|ANDROID_NDK|cross_compile' "$WORKFLOW"; then
  fail qualification_workflow_must_consume_not_rebuild
fi
if grep -Eiq -- '(^|[;&|[:space:]])docker[[:space:]]+(build|pull)|(^|[;&|[:space:]])(apt|apt-get|pkg)[[:space:]]+(install|update|upgrade)|(^|[;&|[:space:]])dpkg[[:space:]]+(--unpack|--install|-i|--configure)' "$WORKFLOW"; then
  fail qualification_workflow_must_replay_without_runtime_construction
fi

for option in \
  --runtime-archive \
  --runtime-package-lock \
  --runtime-snapshot \
  --runtime-replay
do
  [[ "$(grep -Fc "parser.add_argument(\"$option\", required=True)" "$PACKAGER")" -eq 1 ]] \
    || fail "packager runtime input is not mandatory: $option"
done
jq -e '
  (.required | index("retainedRuntime")) != null
  and .properties.retainedRuntime."$ref" == "#/$defs/retainedRuntime"
  and ."$defs".retainedRuntime.additionalProperties == false
  and (.properties.gates.required | length) == 7
' "$QUALIFICATION_SCHEMA" >/dev/null \
  || fail retained_runtime_qualification_schema_contract_missing

printf 'Automated release qualification workflow contract passed\n'
