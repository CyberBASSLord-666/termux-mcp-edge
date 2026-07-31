#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
chmod 700 "$ROOT"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
SCRIPT="$REPO_ROOT/scripts/package_shizuku_rish_test_artifact.sh"
SCHEMA="$REPO_ROOT/docs/android-rish-development-artifact-schema-v1.json"
REAL_PATH="$PATH"

fail_test() {
  printf 'Android rish development artifact test failed: %s\n' "$1" >&2
  exit 1
}

assert_fails() {
  local reason="$1"
  shift
  if "$@" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "command unexpectedly succeeded: $reason"
  fi
  grep -Fq "$reason" "$ROOT/last.stderr" \
    || fail_test "expected failure reason was absent: $reason"
}

mkdir -m 700 "$ROOT/fake-bin" "$ROOT/output"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'printf "%s\n" "ELF 64-bit LSB pie executable, ARM aarch64, for Android 24"' \
  >"$ROOT/fake-bin/file"
chmod 700 "$ROOT/fake-bin/file"

BINARY="$ROOT/candidate"
printf '%s\n' '#!/usr/bin/env bash' 'exit 0' >"$BINARY"
chmod 700 "$BINARY"
CARGO_LOCK="$ROOT/Cargo.lock"
printf '%s\n' '# exact development lock fixture' >"$CARGO_LOCK"
chmod 600 "$CARGO_LOCK"

run_package() {
  PATH="$ROOT/fake-bin:$REAL_PATH" bash "$SCRIPT" \
    --binary "$1" \
    --cargo-lock "$2" \
    --output-dir "$3" \
    --repository CyberBASSLord-666/termux-mcp-edge \
    --commit aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
    --workflow-run-id 1007 \
    --version 0.7.0
}

bash -n "$SCRIPT"
bash -n "${BASH_SOURCE[0]}"
jq -e '
  .type == "object"
  and .additionalProperties == false
  and (.required | length) == 18
  and .properties.artifactClass.const == "android_rish_development_only_v1"
  and .properties.releaseEligible.const == false
  and .properties.productionControlQualified.const == false
  and .properties.artifactName.const == "termux-mcp-server-aarch64-linux-android-android-rish-development"
  and .properties.posture.const == "android-rish-development"
  and .properties.features.const == ["android-rish"]
  and .properties.target.const == "aarch64-linux-android"
' "$SCHEMA" >/dev/null

BUNDLE="$ROOT/output/pass"
run_package "$BINARY" "$CARGO_LOCK" "$BUNDLE" \
  >"$ROOT/pass.stdout" 2>"$ROOT/pass.stderr"
[[ "$(<"$ROOT/pass.stdout")" == '[android-rish-development-artifact] result=PASS' ]] \
  || fail_test success_output_contract_changed
[[ ! -s "$ROOT/pass.stderr" ]] || fail_test successful_package_wrote_stderr
[[ "$(stat -c '%a' "$BUNDLE")" == 700 ]] || fail_test bundle_mode_invalid
[[ "$(stat -c '%a' "$BUNDLE/termux-mcp-server")" == 700 ]] \
  || fail_test binary_mode_invalid
[[ "$(stat -c '%a' "$BUNDLE/SHA256SUMS")" == 600 ]] \
  || fail_test checksum_mode_invalid
[[ "$(stat -c '%a' "$BUNDLE/artifact-manifest.json")" == 600 ]] \
  || fail_test manifest_mode_invalid
[[ "$(find "$BUNDLE" -mindepth 1 -maxdepth 1 -type f | wc -l)" == 3 ]] \
  || fail_test bundle_inventory_invalid
(cd "$BUNDLE" && sha256sum --check --strict SHA256SUMS >/dev/null)

EXPECTED_LOCK_SHA="$(sha256sum "$CARGO_LOCK" | awk '{print $1}')"
jq -e --arg lock_sha "$EXPECTED_LOCK_SHA" '
  (keys == [
    "artifactClass",
    "artifactName",
    "bytes",
    "cargoLockSha256",
    "commit",
    "createdAt",
    "elf",
    "features",
    "fileName",
    "posture",
    "productionControlQualified",
    "releaseEligible",
    "repository",
    "schemaVersion",
    "sha256",
    "target",
    "version",
    "workflowRunId"
  ])
  and .artifactClass == "android_rish_development_only_v1"
  and .releaseEligible == false
  and .productionControlQualified == false
  and .repository == "CyberBASSLord-666/termux-mcp-edge"
  and .commit == "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  and .workflowRunId == "1007"
  and .artifactName == "termux-mcp-server-aarch64-linux-android-android-rish-development"
  and .posture == "android-rish-development"
  and .features == ["android-rish"]
  and .target == "aarch64-linux-android"
  and .fileName == "termux-mcp-server"
  and .version == "0.7.0"
  and .elf == "aarch64-android-elf"
  and .cargoLockSha256 == $lock_sha
' "$BUNDLE/artifact-manifest.json" >/dev/null \
  || fail_test manifest_contract_invalid

SYMLINK="$ROOT/candidate-link"
ln -s "$BINARY" "$SYMLINK"
assert_fails binary_invalid \
  run_package "$SYMLINK" "$CARGO_LOCK" "$ROOT/output/symlink"
[[ ! -e "$ROOT/output/symlink" ]] || fail_test symlink_published_output

LOCK_LINK="$ROOT/Cargo.lock.link"
ln -s "$CARGO_LOCK" "$LOCK_LINK"
assert_fails cargo_lock_invalid \
  run_package "$BINARY" "$LOCK_LINK" "$ROOT/output/lock-link"
[[ ! -e "$ROOT/output/lock-link" ]] || fail_test lock_symlink_published_output

mkdir -m 700 "$ROOT/output/existing"
printf '%s\n' preserve >"$ROOT/output/existing/sentinel"
assert_fails output_directory_invalid \
  run_package "$BINARY" "$CARGO_LOCK" "$ROOT/output/existing"
[[ "$(<"$ROOT/output/existing/sentinel")" == preserve ]] \
  || fail_test existing_output_changed
[[ -z "$(find "$ROOT/output" -maxdepth 1 -name '.android-rish-development.*' -print -quit)" ]] \
  || fail_test failed_package_left_staging_state

printf 'Android rish development artifact package tests passed\n'
