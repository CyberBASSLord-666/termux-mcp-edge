#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
SCRIPT="$ROOT/scripts/cross_compile.sh"
CI_WORKFLOW="$ROOT/.github/workflows/ci.yml"
ANDROID_WORKFLOW="$ROOT/.github/workflows/android-cross-compile.yml"
SECURITY_WORKFLOW="$ROOT/.github/workflows/security.yml"
QUALIFICATION_POLICY="$ROOT/docs/release-qualification-policy-v1.json"
QUALIFICATION_POLICY_SCHEMA="$ROOT/docs/release-qualification-policy-schema-v1.json"
TMP="$(mktemp -d)"
trap 'rm -rf -- "$TMP"' EXIT

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

assert_line() {
  local expected="$1"
  grep -Fxq -- "$expected" "$CARGO_LOG" || fail "missing Cargo invocation: $expected"
}

assert_contains() {
  local expected="$1"
  local file="$2"
  grep -Fq -- "$expected" "$file" || fail "missing lockfile contract marker: $expected"
}

assert_contains 'cargo metadata --locked --all-features --format-version 1 --no-deps' "$CI_WORKFLOW"
assert_contains 'cargo clippy --locked --workspace --all-targets -- -D warnings' "$CI_WORKFLOW"
assert_contains 'cargo clippy --locked --workspace --all-targets --features mcp-runtime -- -D warnings' "$CI_WORKFLOW"
assert_contains 'cargo clippy --locked --workspace --all-targets --all-features -- -D warnings' "$CI_WORKFLOW"
assert_contains 'cargo test --locked --workspace --all-targets' "$CI_WORKFLOW"
assert_contains 'cargo test --locked --workspace --all-targets --features mcp-runtime' "$CI_WORKFLOW"
assert_contains 'cargo test --locked --workspace --all-targets --features full-suite' "$CI_WORKFLOW"
assert_contains 'cargo test --locked --workspace --all-targets --all-features' "$CI_WORKFLOW"
[[ "$(grep -Fc 'git diff --exit-code -- Cargo.toml Cargo.lock' "$CI_WORKFLOW")" -eq 4 ]] \
  || fail ci_dependency_input_brackets_changed

metadata_line="$(grep -nF 'cargo metadata --locked --all-features --format-version 1 --no-deps' "$CI_WORKFLOW" | head -n1 | cut -d: -f1)"
cache_line="$(grep -nF 'uses: Swatinem/rust-cache@' "$CI_WORKFLOW" | head -n1 | cut -d: -f1)"
[[ "$metadata_line" =~ ^[1-9][0-9]*$ && "$cache_line" =~ ^[1-9][0-9]*$ ]]
((metadata_line < cache_line)) || fail locked_metadata_must_precede_cargo_aware_cache

python3 - "$CI_WORKFLOW" <<'PY'
import pathlib
import re
import sys

import yaml

workflow = yaml.safe_load(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
workflow_text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
if "RUST_TEST_THREADS" in workflow_text:
    raise SystemExit("CI workflow may not serialize the Rust test harness")
repository_root = pathlib.Path(sys.argv[1]).resolve().parents[2]
for cargo_config in (
    repository_root / ".cargo" / "config",
    repository_root / ".cargo" / "config.toml",
):
    if cargo_config.exists() and "RUST_TEST_THREADS" in cargo_config.read_text(encoding="utf-8"):
        raise SystemExit(f"{cargo_config} may not serialize the Rust test harness")
job = workflow["jobs"]["rust"]
if job.get("timeout-minutes") != 45:
    raise SystemExit("CI Rust job timeout no longer composes its explicit step bounds")
if any(key in job for key in ("continue-on-error", "if")):
    raise SystemExit("CI Rust job became conditional or fail-open")
for owner, defaults in (
    ("workflow", workflow.get("defaults")),
    ("CI Rust job", job.get("defaults")),
):
    if (
        isinstance(defaults, dict)
        and isinstance(defaults.get("run"), dict)
        and "shell" in defaults["run"]
    ):
        raise SystemExit(f"{owner} may not override the runner shell")
for owner, environment in (
    ("workflow", workflow.get("env")),
    ("CI Rust job", job.get("env")),
):
    if isinstance(environment, dict) and "RUST_TEST_THREADS" in environment:
        raise SystemExit(f"{owner} may not serialize the Rust test harness")
expected = {
    "Tests (default posture)": "timeout --verbose --signal=TERM --kill-after=30s 8m cargo test --locked --workspace --all-targets",
    "Tests (MCP runtime posture)": "timeout --verbose --signal=TERM --kill-after=30s 8m cargo test --locked --workspace --all-targets --features mcp-runtime",
    "Tests (full-suite posture)": "timeout --verbose --signal=TERM --kill-after=30s 8m cargo test --locked --workspace --all-targets --features full-suite",
    "Tests (all features)": "timeout --verbose --signal=TERM --kill-after=30s 8m cargo test --locked --workspace --all-targets --all-features",
}
steps = {
    step.get("name"): step
    for step in job["steps"]
    if step.get("name") in expected
}
if set(steps) != set(expected):
    raise SystemExit("CI test posture set changed")
for name, command in expected.items():
    step = steps[name]
    if step.get("run") != command:
        raise SystemExit(f"{name} lost its exact locked timeout command")
    if any(key in step for key in ("continue-on-error", "if", "shell")):
        raise SystemExit(f"{name} became conditional, fail-open, or shell-overridden")
    if isinstance(step.get("env"), dict) and "RUST_TEST_THREADS" in step["env"]:
        raise SystemExit(f"{name} may not serialize the Rust test harness")
    lowered = command.lower()
    if "--test-threads" in lowered or "nextest" in lowered or "retry" in lowered:
        raise SystemExit(f"{name} changed normal one-pass parallel test semantics")
all_runs = "\n".join(
    str(step.get("run", ""))
    for step in job["steps"]
)
if len(re.findall(r"\bcargo\s+test\b", all_runs)) != 4:
    raise SystemExit("CI must invoke exactly four locked Cargo test postures")
PY

assert_contains 'cargo metadata --locked --format-version 1 --no-deps' "$ANDROID_WORKFLOW"
assert_contains 'git diff --exit-code -- Cargo.toml Cargo.lock' "$ANDROID_WORKFLOW"
for protected_runtime_input in \
  'scripts/termux_device_smoke.sh' \
  'scripts/termux_deploy.sh'
do
  [[ "$(grep -Fc -- "- \"$protected_runtime_input\"" "$ANDROID_WORKFLOW")" == 1 ]] \
    || fail "protected runtime Android trigger missing or duplicated: $protected_runtime_input"
done
[[ "$(grep -Fc -- '- "scripts/commit_verified_file.py"' "$ANDROID_WORKFLOW")" == 1 ]] \
  || fail commit_helper_android_trigger_missing_or_duplicated
[[ "$(grep -Fc -- '- "scripts/commit_verified_file.py"' "$SECURITY_WORKFLOW")" == 1 ]] \
  || fail commit_helper_security_trigger_missing_or_duplicated
for runtime_input in \
  'scripts/verify_runtime_snapshot.sh' \
  'docs/runtime-package-lock-schema-v*.json' \
  'docs/runtime-snapshot-schema-v*.json' \
  'docs/runtime-snapshot-replay-schema-v*.json'
do
  [[ "$(grep -Fc -- "- \"$runtime_input\"" "$ANDROID_WORKFLOW")" == 1 ]] \
    || fail "runtime snapshot Android trigger missing or duplicated: $runtime_input"
done
python3 - \
  "$ANDROID_WORKFLOW" \
  "$QUALIFICATION_POLICY" \
  "$QUALIFICATION_POLICY_SCHEMA" <<'PY'
import json
import pathlib
import sys

workflow = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
policy = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
schema = json.loads(pathlib.Path(sys.argv[3]).read_text(encoding="utf-8"))
expected = ["file", "jq", "python", "termux-services"]
dockerfile_start = workflow.index(
    "cat >\"$runtime_context/Dockerfile\" <<'DOCKERFILE'"
)
dockerfile_end = workflow.index("\n          DOCKERFILE", dockerfile_start)
dockerfile = workflow[dockerfile_start:dockerfile_end]
if "apt-get update" in dockerfile:
    raise SystemExit("qualified runtime final build still resolves a live repository")
for marker in (
    "COPY --chown=0:0 package-inputs/debs/",
    "COPY --chown=0:0 package-inputs/indexes/",
    "termux-runtime-package-lock-v1.json",
    "--no-download",
    "/share/termux-mcp/runtime-packages/*.deb",
    "/share/termux-mcp/runtime-repository-indexes/",
):
    if marker not in dockerfile:
        raise SystemExit(f"offline qualified runtime Dockerfile missing: {marker}")
for forbidden in (
    "rm -rf /data/data/com.termux/files/usr/share/termux-mcp/runtime-packages",
    "rm -rf /data/data/com.termux/files/usr/share/termux-mcp/runtime-repository-indexes",
):
    if forbidden in dockerfile:
        raise SystemExit(f"qualified runtime deletes retained provenance: {forbidden}")
for marker in (
    'chmod 0555 \\\n            "$runtime_context/package-inputs/debs"',
    'find "$runtime_context/package-inputs" -type f -exec chmod 0444',
    'chmod 0444 "$runtime_context/termux-runtime-package-lock-v1.json"',
):
    if marker not in workflow:
        raise SystemExit(f"retained provenance is not immutable in image: {marker}")
build_start = workflow.index("docker build \\", dockerfile_end)
build_end = workflow.index('"$runtime_context"', build_start)
build = workflow[build_start:build_end]
for marker in ("--pull=false", "--no-cache", "--network=none", "--iidfile"):
    if marker not in build:
        raise SystemExit(f"qualified runtime build is not closed: {marker}")
for marker in (
    "--download-only",
    "file jq python termux-services",
    "repositoryMetadataAuthenticated",
    "packageBytesFrozenBeforeBuild",
    '"finalImageBuildNetwork": "none"',
    "dpkg-deb",
):
    if marker not in workflow:
        raise SystemExit(f"runtime package freeze contract missing: {marker}")
if policy["environmentRequirements"]["runtimePackages"] != expected:
    raise SystemExit("qualification policy runtime package list changed")
schema_packages = schema["properties"]["environmentRequirements"][
    "properties"
]["runtimePackages"]["const"]
if schema_packages != expected:
    raise SystemExit("qualification policy schema runtime package list changed")
PY
python3 - "$ANDROID_WORKFLOW" <<'PY'
import sys
import yaml

workflow = yaml.safe_load(open(sys.argv[1], encoding="utf-8"))
workflow_defaults = workflow.get("defaults")
if (
    isinstance(workflow_defaults, dict)
    and isinstance(workflow_defaults.get("run"), dict)
    and "shell" in workflow_defaults["run"]
):
    raise SystemExit("workflow may not override the runner shell")
for job_name in ("android-aarch64", "termux-emulated"):
    job = workflow["jobs"][job_name]
    if "continue-on-error" in job:
        raise SystemExit(f"{job_name} job may not ignore failure")
    job_defaults = job.get("defaults")
    if (
        isinstance(job_defaults, dict)
        and isinstance(job_defaults.get("run"), dict)
        and "shell" in job_defaults["run"]
    ):
        raise SystemExit(f"{job_name} job may not override the runner shell")
    for index, step in enumerate(job["steps"]):
        if "continue-on-error" in step or "if" in step:
            raise SystemExit(
                f"{job_name} step {step.get('name', index)} is conditional or fail-open"
            )
        if "shell" in step:
            raise SystemExit(
                f"{job_name} step {step.get('name', index)} may not override the runner shell"
            )
native_job = workflow["jobs"]["termux-emulated"]
if native_job.get("timeout-minutes") != 75:
    raise SystemExit("native Termux job timeout no longer composes polling and validation")
steps = workflow["jobs"]["termux-emulated"]["steps"]
names = [step.get("name") for step in steps]
companion = steps[names.index("Resolve exact companion workflow evidence")]["run"]
if "readonly max_wait_seconds=2700" not in companion:
    raise SystemExit("native companion-run polling budget changed")
start = names.index("Validate emulated evidence")
if names[start : start + 3] != [
    "Validate emulated evidence",
    "Freeze native qualification components",
    "Upload native qualification components",
]:
    raise SystemExit("validate/freeze/upload qualification ordering changed")
upload = steps[start + 2]
expected_upload = {
    "name": "termux-mcp-native-qualification-components",
    "path": "${{ steps.components.outputs.root }}/",
    "if-no-files-found": "error",
    "include-hidden-files": False,
    "compression-level": 0,
    "overwrite": False,
    "retention-days": 30,
}
if upload.get("uses") != "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a":
    raise SystemExit("qualification component upload action pin changed")
if upload.get("with") != expected_upload:
    raise SystemExit("qualification component upload options changed")

runtime_start = names.index("Freeze replayable qualified runtime snapshot")
if names[runtime_start : runtime_start + 2] != [
    "Freeze replayable qualified runtime snapshot",
    "Upload replayable qualified runtime snapshot",
]:
    raise SystemExit("runtime snapshot freeze/upload ordering changed")
if runtime_start != start + 3:
    raise SystemExit("runtime snapshot must follow validated component publication")
runtime_upload = steps[runtime_start + 1]
if runtime_upload.get("uses") != "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a":
    raise SystemExit("runtime snapshot upload action pin changed")
runtime_with = runtime_upload.get("with")
expected_runtime_paths = "\n".join(
    (
        "${{ steps.runtime-snapshot.outputs.root }}/termux-qualified-runtime-image-v1.tar.gz",
        "${{ steps.runtime-snapshot.outputs.root }}/termux-runtime-package-lock-v1.json",
        "${{ steps.runtime-snapshot.outputs.root }}/termux-runtime-snapshot-v1.json",
    )
) + "\n"
if runtime_with != {
    "name": "termux-mcp-qualified-runtime-snapshot",
    "path": expected_runtime_paths,
    "if-no-files-found": "error",
    "include-hidden-files": False,
    "compression-level": 0,
    "overwrite": False,
    "retention-days": 30,
}:
    raise SystemExit("runtime snapshot upload options changed")
freeze_script = steps[runtime_start]["run"]
for marker in (
    'docker save "$runtime_tag" | gzip -n -9',
    "--network none",
    '"rebuildReproducibilityClaim": False',
    'test "$(find "$snapshot" -mindepth 1 -maxdepth 1 -type f | wc -l)" = 3',
):
    if marker not in freeze_script:
        raise SystemExit(f"runtime snapshot freeze contract missing: {marker}")
PY

mkdir -p "$TMP/bin" "$TMP/ndk/toolchains/llvm/prebuilt/linux-x86_64/bin"
CARGO_LOG="$TMP/cargo.log"
export CARGO_LOG

cat >"$TMP/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$CARGO_LOG"
locked=false
for argument in "$@"; do
  [[ "$argument" == --locked ]] && locked=true
done
if [[ "$locked" != true ]]; then
  printf 'mock cargo rejected unlocked invocation\n' >&2
  exit 70
fi
if [[ "${CARGO_MOCK_FAIL_LOCKED:-false}" == true ]]; then
  printf 'mock cargo injected locked-build failure\n' >&2
  exit 72
fi
exit 0
EOF

cat >"$TMP/bin/rustup" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$*" == 'target list --installed' ]]; then
  printf 'aarch64-linux-android\n'
  exit 0
fi
printf 'unexpected rustup invocation: %s\n' "$*" >&2
exit 71
EOF

chmod 755 "$TMP/bin/cargo" "$TMP/bin/rustup"
touch "$TMP/ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android24-clang"
touch "$TMP/ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar"
chmod 755 \
  "$TMP/ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android24-clang" \
  "$TMP/ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar"

PATH="$TMP/bin:$PATH" \
ANDROID_NDK_HOME="$TMP/ndk" \
BUILD_FEATURES='' \
  bash "$SCRIPT" >"$TMP/default.out"
assert_line 'build --release --locked --target aarch64-linux-android'

: >"$CARGO_LOG"
PATH="$TMP/bin:$PATH" \
ANDROID_NDK_HOME="$TMP/ndk" \
BUILD_FEATURES=mcp-runtime \
  bash "$SCRIPT" >"$TMP/feature.out"
assert_line 'build --release --locked --target aarch64-linux-android --features mcp-runtime'

[[ "$(wc -l <"$CARGO_LOG")" -eq 1 ]] || fail unexpected_feature_build_count
grep -Fq 'Building default feature posture' "$TMP/default.out" || fail default_posture_log_missing
grep -Fq 'Building explicit feature posture: mcp-runtime' "$TMP/feature.out" \
  || fail feature_posture_log_missing

for posture in default feature; do
  : >"$CARGO_LOG"
  features=''
  [[ "$posture" == feature ]] && features=mcp-runtime
  set +e
  PATH="$TMP/bin:$PATH" \
  ANDROID_NDK_HOME="$TMP/ndk" \
  BUILD_FEATURES="$features" \
  CARGO_MOCK_FAIL_LOCKED=true \
    bash "$SCRIPT" >"$TMP/$posture.failure.out" 2>&1
  status=$?
  set -e
  [[ "$status" -eq 72 ]] || fail "$posture branch masked Cargo failure with status $status"
  grep -Fq 'mock cargo injected locked-build failure' "$TMP/$posture.failure.out" \
    || fail "$posture branch did not execute the locked Cargo failure path"
  if grep -Fq 'Binary ready at:' "$TMP/$posture.failure.out"; then
    fail "$posture branch claimed a binary after Cargo failed"
  fi
done

printf 'Cross-compile lockfile contract passed\n'
