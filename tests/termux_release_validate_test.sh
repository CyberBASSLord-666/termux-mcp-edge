#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

ROOT="$(mktemp -d)"
LISTENER_PID=""

cleanup_test() {
  if [[ -n "$LISTENER_PID" ]]; then
    kill "$LISTENER_PID" >/dev/null 2>&1 || true
    wait "$LISTENER_PID" >/dev/null 2>&1 || true
  fi
  rm -rf -- "$ROOT"
}
trap cleanup_test EXIT INT TERM
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/termux_release_validate.sh"
SCHEMA="$REPO_ROOT/docs/release-evidence-schema-v1.json"
REAL_PATH="$PATH"
REAL_TIMEOUT="$(command -v timeout)"
REAL_SHA256SUM="$(command -v sha256sum)"

fail_test() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

assert_eq() {
  [[ "$1" == "$2" ]] || fail_test "expected '$2', got '$1'"
}

assert_fails() {
  if "$@" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "command unexpectedly succeeded"
  fi
}

assert_no_private_output() {
  local label="$1" file
  shift
  for file in "$@"; do
    if grep -Fq "$ROOT" "$file" \
      || grep -Fq fixture-private-token "$file" \
      || grep -Fq fixture-session "$file" \
      || grep -Fq outside-private-content "$file"; then
      fail_test "$label exposed private validation data"
    fi
  done
}

assert_report_contract() {
  jq -e '
    (keys == ["artifacts","completedAt","environment","failureCode","phases","releaseEligible","repository","requestedPhase","results","schemaVersion","startedAt","status","sustainedObservation","validatorVersion"])
    and .schemaVersion == 1
    and (.validatorVersion | type == "string" and test("^[0-9]+$"))
    and (.status == "pass" or .status == "fail" or .status == "fixture")
    and (.startedAt | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"))
    and (.completedAt | type == "string")
    and (.repository | keys == ["androidRunId","ciRunId","commit","securityRunId","version"])
    and (.repository.commit | test("^[0-9a-f]{40}$"))
    and (.environment | keys == ["architecture","fixtureMode","tools"])
    and (.environment.tools | keys == ["bash","curl","file","jq"])
    and (.artifacts | keys == ["androidVolumeControl","baseline","default","mcpRuntime"])
    and (.phases | keys == ["deployment","preflight","runtime"])
    and (.results | type == "array" and length <= 256)
    and (all(.results[]; (keys == ["check","code","outcome","phase"])))
    and (.sustainedObservation | keys == ["minimumMinutes","minutes","operatorSupplied","reasonCode","status"])
    and .sustainedObservation.operatorSupplied == true
    and .sustainedObservation.minimumMinutes == 60
    and (if .status == "fail" then (.failureCode | type == "string") and (.releaseEligible == false)
         elif .status == "fixture" then (.failureCode == null and .releaseEligible == false and .environment.fixtureMode == true)
         else (.failureCode == null and .environment.fixtureMode == false)
         end)
    and (if .releaseEligible then
           .status == "pass"
           and .requestedPhase == "all"
           and .phases == {preflight:"pass",runtime:"pass",deployment:"pass"}
           and .sustainedObservation.status == "pass"
           and .sustainedObservation.minutes >= 60
           and .sustainedObservation.reasonCode == "stable"
         else true end)
  ' "$1" >/dev/null
}

make_artifact() {
  local path="$1" version="$2" posture="$3"
  cat >"$path" <<EOF
#!/usr/bin/env bash
set -euo pipefail
if [[ "\${1:-}" == --version ]]; then
  printf 'termux-mcp-server %s\\n' '$version'
  exit 0
fi
printf '%s\\n' '$posture' >>'$ROOT/unexpected-artifact-start'
exit 99
EOF
  chmod 700 "$path"
}

make_runtime_artifact() {
  local path="$1" version="$2" posture="$3"
  cat >"$path" <<EOF
#!/usr/bin/env bash
set -euo pipefail
if [[ "\${1:-}" == --version ]]; then
  printf 'termux-mcp-server %s\\n' '$version'
  exit 0
fi
if [[ '$posture' == mcp && "\${MCP__ANDROID__VOLUME_CONTROL_ENABLED:-false}" == true ]]; then
  printf '%s\n' 'MCP__ANDROID__VOLUME_CONTROL_ENABLED requires a binary built with the android-volume-control feature' >&2
  exit 1
fi
if [[ "\${1:-}" == --issue-create-directory-grant ]]; then
  exec python3 '$REPO_ROOT/tests/fixtures/release_validator_mock_server.py' issue
fi
exec python3 '$REPO_ROOT/tests/fixtures/release_validator_mock_server.py' '$posture' '$version'
EOF
  chmod 700 "$path"
}

make_hanging_artifact() {
  local path="$1"
  cat >"$path" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == --version ]]; then
  sleep 30
  exit 0
fi
exit 99
EOF
  chmod 700 "$path"
}

sha() {
  sha256sum -- "$1" | awk '{print $1}'
}

write_manifest() {
  local path="$1" artifact="$2" digest="$3" posture="$4" artifact_name features
  local bytes
  bytes="$(stat -c '%s' "$artifact")"
  case "$posture" in
    default)
      artifact_name=termux-mcp-server-aarch64-linux-android-default
      features='[]'
      ;;
    mcp-runtime)
      artifact_name=termux-mcp-server-aarch64-linux-android-mcp-runtime
      features='["mcp-runtime"]'
      ;;
    android-volume-control)
      artifact_name=termux-mcp-server-aarch64-linux-android-android-volume-control
      features='["android-volume-control"]'
      ;;
    *) fail_test "unknown manifest posture" ;;
  esac
  jq -n \
    --arg artifact_name "$artifact_name" \
    --arg posture "$posture" \
    --arg sha "$digest" \
    --argjson bytes "$bytes" \
    --argjson features "$features" '
      {
        schemaVersion: 1,
        repository: "CyberBASSLord-666/termux-mcp-edge",
        commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        workflowRunId: "1003",
        artifactName: $artifact_name,
        posture: $posture,
        features: $features,
        target: "aarch64-linux-android",
        fileName: "termux-mcp-server",
        version: "0.5.1",
        sha256: $sha,
        bytes: $bytes,
        elf: "aarch64-android-elf",
        createdAt: "2026-07-11T00:00:00Z"
      }
    ' >"$path"
  chmod 600 "$path"
}

write_config() {
  local path="$1" default_artifact="$2" default_sha="$3" mcp_artifact="$4" mcp_sha="$5"
  local sustained_status="${6:-not_run}" sustained_minutes="${7:-0}" sustained_reason="${8:-not_observed}"
  local default_manifest="${path}.default-manifest.json" mcp_manifest="${path}.mcp-manifest.json" volume_control_manifest="${path}.volume-control-manifest.json"
  write_manifest "$default_manifest" "$default_artifact" "$default_sha" default
  write_manifest "$mcp_manifest" "$mcp_artifact" "$mcp_sha" mcp-runtime
  write_manifest "$volume_control_manifest" "$VOLUME_CONTROL_ARTIFACT" "$VOLUME_CONTROL_SHA" android-volume-control
  cat >"$path" <<EOF
EXPECTED_COMMIT=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
EXPECTED_VERSION=0.5.1
DEFAULT_ARTIFACT=$default_artifact
DEFAULT_SHA256=$default_sha
DEFAULT_MANIFEST=$default_manifest
MCP_ARTIFACT=$mcp_artifact
MCP_SHA256=$mcp_sha
MCP_MANIFEST=$mcp_manifest
VOLUME_CONTROL_ARTIFACT=$VOLUME_CONTROL_ARTIFACT
VOLUME_CONTROL_SHA256=$VOLUME_CONTROL_SHA
VOLUME_CONTROL_MANIFEST=$volume_control_manifest
BASELINE_ARTIFACT=$BASELINE_ARTIFACT
BASELINE_VERSION=0.5.0
BASELINE_SHA256=$BASELINE_SHA
AUTH_TOKEN_FILE=$TOKEN_FILE
SAFE_ROOT=$SAFE_ROOT
BIND_HOST=127.0.0.1
PORT=18765
DEPLOY_SCRIPT=$REPO_ROOT/scripts/termux_deploy.sh
CI_RUN_ID=1001
SECURITY_RUN_ID=1002
ANDROID_RUN_ID=1003
SUSTAINED_OBSERVATION_STATUS=$sustained_status
SUSTAINED_OBSERVATION_MINUTES=$sustained_minutes
SUSTAINED_OBSERVATION_REASON_CODE=$sustained_reason
EOF
  chmod 600 "$path"
}

run_validator() {
  HOME="$HOME" PREFIX="$PREFIX" PATH="$FAKE_BIN:$REAL_PATH" \
    TERMUX_MCP_RELEASE_VALIDATOR_TEST_MODE=1 \
    bash "$SCRIPT" "$@"
}

bash -n "$SCRIPT"
grep -Fq 'valid_capability_grant()' "$SCRIPT"
if grep -Fq -- '{260}' "$SCRIPT"; then
  fail_test "release validator uses a non-portable ERE repetition above Android RE_DUP_MAX"
fi
jq -e '
  .["$schema"] == "https://json-schema.org/draft/2020-12/schema"
  and .additionalProperties == false
  and (.allOf | length) == 5
  and .properties.schemaVersion.const == 1
  and .properties.status.enum == ["pass","fail","fixture"]
  and .properties.artifacts.properties.androidVolumeControl."$ref" == "#/$defs/artifact"
  and (.properties.sustainedObservation.allOf | length) == 3
  and .properties.sustainedObservation.properties.minimumMinutes.const == 60
' "$SCHEMA" >/dev/null
mkdir -p "$ROOT/home/safe" "$ROOT/prefix/bin" "$ROOT/fake-bin"
chmod 700 "$ROOT/home" "$ROOT/home/safe" "$ROOT/prefix" "$ROOT/fake-bin"
HOME="$ROOT/home"
PREFIX="$ROOT/prefix"
FAKE_BIN="$ROOT/fake-bin"
SAFE_ROOT="$HOME/safe"
cp -L -- /bin/sh "$PREFIX/bin/sh"
chmod 700 "$PREFIX/bin/sh"

cat >"$FAKE_BIN/file" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
target="${*: -1}"
if grep -Fq wrong-arch "$target"; then
  printf '%s\n' 'ELF 64-bit LSB executable, x86-64, for GNU/Linux'
else
  printf '%s\n' 'ELF 64-bit LSB pie executable, ARM aarch64, for Android 24'
fi
EOF
chmod 700 "$FAKE_BIN/file"

DEFAULT_ARTIFACT="$ROOT/default-artifact"
MCP_ARTIFACT="$ROOT/mcp-artifact"
VOLUME_CONTROL_ARTIFACT="$ROOT/volume-control-artifact"
BASELINE_ARTIFACT="$ROOT/baseline-artifact"
make_artifact "$DEFAULT_ARTIFACT" 0.5.1 default
make_artifact "$MCP_ARTIFACT" 0.5.1 mcp
make_artifact "$VOLUME_CONTROL_ARTIFACT" 0.5.1 android-volume-control
make_artifact "$BASELINE_ARTIFACT" 0.5.0 baseline
DEFAULT_SHA="$(sha "$DEFAULT_ARTIFACT")"
MCP_SHA="$(sha "$MCP_ARTIFACT")"
VOLUME_CONTROL_SHA="$(sha "$VOLUME_CONTROL_ARTIFACT")"
BASELINE_SHA="$(sha "$BASELINE_ARTIFACT")"

TOKEN_FILE="$ROOT/token"
printf '%s' 'fixture-private-token' >"$TOKEN_FILE"
chmod 600 "$TOKEN_FILE"

CONFIG="$ROOT/preflight.env"
REPORT="$ROOT/preflight.json"
write_config "$CONFIG" "$DEFAULT_ARTIFACT" "$DEFAULT_SHA" "$MCP_ARTIFACT" "$MCP_SHA"
if ! run_validator --config "$CONFIG" --report "$REPORT" --phase preflight >"$ROOT/preflight.stdout" 2>"$ROOT/preflight.stderr"; then
  sed -n '1,160p' "$ROOT/preflight.stdout" >&2
  sed -n '1,160p' "$ROOT/preflight.stderr" >&2
  [[ ! -f "$REPORT" ]] || jq . "$REPORT" >&2
  fail_test "valid preflight failed"
fi

jq -e '
  .schemaVersion == 1
  and .validatorVersion == "4"
  and .status == "fixture"
  and .releaseEligible == false
  and .phases.preflight == "pass"
  and .phases.runtime == "not_run"
  and .phases.deployment == "not_run"
  and .artifacts.default.sha256 != .artifacts.mcpRuntime.sha256
  and .artifacts.androidVolumeControl.sha256 != .artifacts.default.sha256
  and .artifacts.androidVolumeControl.sha256 != .artifacts.mcpRuntime.sha256
  and .artifacts.default.version == "0.5.1"
  and .artifacts.androidVolumeControl.version == "0.5.1"
  and .environment.fixtureMode == true
' "$REPORT" >/dev/null
[[ "$(stat -c '%a' "$REPORT")" == 600 ]] || fail_test "report mode is not 600"
[[ ! -e "$ROOT/unexpected-artifact-start" ]] || fail_test "preflight started an artifact"
if grep -Fq "$ROOT" "$REPORT" || grep -Fq fixture-private-token "$REPORT"; then
  fail_test "preflight report exposed a path or token"
fi
assert_no_private_output "preflight output" "$ROOT/preflight.stdout" "$ROOT/preflight.stderr"

COLLISION_REPORT="$ROOT/report-collision.json"
if HOME="$HOME" PREFIX="$PREFIX" PATH="$FAKE_BIN:$REAL_PATH" \
  TERMUX_MCP_RELEASE_VALIDATOR_TEST_MODE=1 \
  TERMUX_MCP_RELEASE_VALIDATOR_TEST_CREATE_REPORT_COLLISION=1 \
  bash "$SCRIPT" \
    --config "$CONFIG" \
    --report "$COLLISION_REPORT" \
    --phase preflight >"$ROOT/report-collision.stdout" 2>"$ROOT/report-collision.stderr"; then
  fail_test "report publication collision unexpectedly succeeded"
fi
[[ "$(<"$COLLISION_REPORT")" == preserve-existing-report ]] || fail_test "report collision overwrote the destination"
[[ "$(stat -c '%a' "$COLLISION_REPORT")" == 600 ]] || fail_test "report collision changed destination mode"
grep -Fq 'result=FAIL code=report_write_failed' "$ROOT/report-collision.stdout" || fail_test "report collision failure code absent"
[[ -z "$(find "$ROOT" -maxdepth 1 -name '.termux-mcp-release-evidence.*' -print -quit)" ]] || fail_test "report collision left staging state"
assert_no_private_output "report collision output" "$ROOT/report-collision.stdout" "$ROOT/report-collision.stderr"

BAD_CONFIG="$ROOT/digest-mismatch.env"
BAD_REPORT="$ROOT/digest-mismatch.json"
write_config "$BAD_CONFIG" "$DEFAULT_ARTIFACT" "$(printf '0%.0s' $(seq 1 64))" "$MCP_ARTIFACT" "$MCP_SHA"
assert_fails run_validator --config "$BAD_CONFIG" --report "$BAD_REPORT" --phase preflight
jq -e '.status == "fail" and .failureCode == "default_artifact_digest_mismatch" and .phases.preflight == "fail"' "$BAD_REPORT" >/dev/null
if grep -Fq "$ROOT" "$BAD_REPORT" || grep -Fq fixture-private-token "$BAD_REPORT"; then
  fail_test "failure report exposed a path or token"
fi
assert_no_private_output "failure output" "$ROOT/last.stdout" "$ROOT/last.stderr"

MANIFEST_CONFIG="$ROOT/manifest-mismatch.env"
MANIFEST_REPORT="$ROOT/manifest-mismatch.json"
write_config "$MANIFEST_CONFIG" "$DEFAULT_ARTIFACT" "$DEFAULT_SHA" "$MCP_ARTIFACT" "$MCP_SHA"
jq '.commit = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"' \
  "$MANIFEST_CONFIG.default-manifest.json" >"$ROOT/manifest-mismatch.next"
mv "$ROOT/manifest-mismatch.next" "$MANIFEST_CONFIG.default-manifest.json"
chmod 600 "$MANIFEST_CONFIG.default-manifest.json"
assert_fails run_validator --config "$MANIFEST_CONFIG" --report "$MANIFEST_REPORT" --phase preflight
jq -e '.status == "fail" and .failureCode == "default_manifest_mismatch"' "$MANIFEST_REPORT" >/dev/null

VOLUME_MANIFEST_CONFIG="$ROOT/volume-manifest-mismatch.env"
VOLUME_MANIFEST_REPORT="$ROOT/volume-manifest-mismatch.json"
write_config "$VOLUME_MANIFEST_CONFIG" "$DEFAULT_ARTIFACT" "$DEFAULT_SHA" "$MCP_ARTIFACT" "$MCP_SHA"
jq '.posture = "android-volume-status"' \
  "$VOLUME_MANIFEST_CONFIG.volume-control-manifest.json" >"$ROOT/volume-manifest-mismatch.next"
mv "$ROOT/volume-manifest-mismatch.next" "$VOLUME_MANIFEST_CONFIG.volume-control-manifest.json"
chmod 600 "$VOLUME_MANIFEST_CONFIG.volume-control-manifest.json"
assert_fails run_validator --config "$VOLUME_MANIFEST_CONFIG" --report "$VOLUME_MANIFEST_REPORT" --phase preflight
jq -e '.status == "fail" and .failureCode == "android_volume_control_manifest_mismatch"' "$VOLUME_MANIFEST_REPORT" >/dev/null

MISSING_MANIFEST_CONFIG="$ROOT/missing-manifest.env"
MISSING_MANIFEST_REPORT="$ROOT/missing-manifest.json"
grep -v '^DEFAULT_MANIFEST=' "$CONFIG" >"$MISSING_MANIFEST_CONFIG"
chmod 600 "$MISSING_MANIFEST_CONFIG"
assert_fails run_validator --config "$MISSING_MANIFEST_CONFIG" --report "$MISSING_MANIFEST_REPORT" --phase preflight
jq -e '.status == "fail" and .failureCode == "default_manifest_invalid"' "$MISSING_MANIFEST_REPORT" >/dev/null

MISSING_VOLUME_CONFIG="$ROOT/missing-volume-control.env"
MISSING_VOLUME_REPORT="$ROOT/missing-volume-control.json"
grep -v '^VOLUME_CONTROL_' "$CONFIG" >"$MISSING_VOLUME_CONFIG"
chmod 600 "$MISSING_VOLUME_CONFIG"
assert_fails run_validator --config "$MISSING_VOLUME_CONFIG" --report "$MISSING_VOLUME_REPORT" --phase preflight
[[ ! -e "$MISSING_VOLUME_REPORT" ]] || fail_test "missing volume-control metadata unexpectedly produced a report"
grep -Fq artifact_digest_metadata_invalid "$ROOT/last.stderr" || fail_test "missing volume-control metadata was not rejected"

MUTATING_ARTIFACT="$ROOT/mutating-artifact"
make_artifact "$MUTATING_ARTIFACT" 0.5.1 default
MUTATING_CONFIG="$ROOT/mutating.env"
MUTATING_REPORT="$ROOT/mutating.json"
write_config "$MUTATING_CONFIG" "$MUTATING_ARTIFACT" "$(sha "$MUTATING_ARTIFACT")" "$MCP_ARTIFACT" "$MCP_SHA"
cat >"$FAKE_BIN/sha256sum" <<EOF
#!/usr/bin/env bash
set -euo pipefail
'$REAL_SHA256SUM' "\$@"
if [[ "\${*: -1}" == '$MUTATING_ARTIFACT' ]]; then
  printf '%s\n' '# changed-after-digest' >>'$MUTATING_ARTIFACT'
fi
EOF
chmod 700 "$FAKE_BIN/sha256sum"
assert_fails run_validator --config "$MUTATING_CONFIG" --report "$MUTATING_REPORT" --phase preflight
jq -e '.status == "fail" and .failureCode == "artifact_changed_during_pinning"' "$MUTATING_REPORT" >/dev/null
rm -f -- "$FAKE_BIN/sha256sum"

INJECTION_MARKER="$ROOT/config-evaluation-marker"
INJECTION_ARTIFACT="\$(touch $INJECTION_MARKER)"
INJECTION_CONFIG="$ROOT/literal-config.env"
INJECTION_REPORT="$ROOT/literal-config.json"
write_config "$INJECTION_CONFIG" "$DEFAULT_ARTIFACT" "$DEFAULT_SHA" "$MCP_ARTIFACT" "$MCP_SHA"
sed -i "s|^DEFAULT_ARTIFACT=.*$|DEFAULT_ARTIFACT=$INJECTION_ARTIFACT|" "$INJECTION_CONFIG"
assert_fails run_validator --config "$INJECTION_CONFIG" --report "$INJECTION_REPORT" --phase preflight
jq -e '.status == "fail" and .failureCode == "default_artifact_invalid"' "$INJECTION_REPORT" >/dev/null
[[ ! -e "$INJECTION_MARKER" ]] || fail_test "configuration value was evaluated as shell code"

DUPLICATE_CONFIG="$ROOT/duplicate.env"
DUPLICATE_REPORT="$ROOT/duplicate.json"
cp -- "$CONFIG" "$DUPLICATE_CONFIG"
printf '%s\n' 'PORT=18766' >>"$DUPLICATE_CONFIG"
chmod 600 "$DUPLICATE_CONFIG"
assert_fails run_validator --config "$DUPLICATE_CONFIG" --report "$DUPLICATE_REPORT" --phase preflight
[[ ! -e "$DUPLICATE_REPORT" ]] || fail_test "duplicate configuration unexpectedly produced a report"
grep -Fq config_key_duplicate "$ROOT/last.stderr" || fail_test "duplicate configuration was not rejected"

PUBLIC_CONFIG="$ROOT/public-config.env"
PUBLIC_CONFIG_REPORT="$ROOT/public-config.json"
cp -- "$CONFIG" "$PUBLIC_CONFIG"
chmod 640 "$PUBLIC_CONFIG"
assert_fails run_validator --config "$PUBLIC_CONFIG" --report "$PUBLIC_CONFIG_REPORT" --phase preflight
[[ ! -e "$PUBLIC_CONFIG_REPORT" ]] || fail_test "nonprivate configuration unexpectedly produced a report"
grep -Fq config_not_private_regular_file "$ROOT/last.stderr" || fail_test "nonprivate configuration was not rejected"

OVERSIZED_CONFIG="$ROOT/oversized-config.env"
OVERSIZED_CONFIG_REPORT="$ROOT/oversized-config.json"
cp -- "$CONFIG" "$OVERSIZED_CONFIG"
awk 'BEGIN {printf "#"; for (i = 0; i < 66000; i++) printf "x"; printf "\n"}' >>"$OVERSIZED_CONFIG"
chmod 600 "$OVERSIZED_CONFIG"
assert_fails run_validator --config "$OVERSIZED_CONFIG" --report "$OVERSIZED_CONFIG_REPORT" --phase preflight
[[ ! -e "$OVERSIZED_CONFIG_REPORT" ]] || fail_test "oversized configuration unexpectedly produced a report"
grep -Fq config_size_invalid "$ROOT/last.stderr" || fail_test "oversized configuration was not rejected"

CR_CONFIG="$ROOT/carriage-return.env"
CR_CONFIG_REPORT="$ROOT/carriage-return.json"
cp -- "$CONFIG" "$CR_CONFIG"
printf '# invalid\r\n' >>"$CR_CONFIG"
chmod 600 "$CR_CONFIG"
assert_fails run_validator --config "$CR_CONFIG" --report "$CR_CONFIG_REPORT" --phase preflight
[[ ! -e "$CR_CONFIG_REPORT" ]] || fail_test "carriage-return configuration unexpectedly produced a report"
grep -Fq config_carriage_return "$ROOT/last.stderr" || fail_test "carriage-return configuration was not rejected"

WRONG_ARCH="$ROOT/wrong-arch-artifact"
make_artifact "$WRONG_ARCH" 0.5.1 default
printf '%s\n' '# wrong-arch' >>"$WRONG_ARCH"
WRONG_CONFIG="$ROOT/wrong-arch.env"
WRONG_REPORT="$ROOT/wrong-arch.json"
write_config "$WRONG_CONFIG" "$WRONG_ARCH" "$(sha "$WRONG_ARCH")" "$MCP_ARTIFACT" "$MCP_SHA"
assert_fails run_validator --config "$WRONG_CONFIG" --report "$WRONG_REPORT" --phase preflight
jq -e '.status == "fail" and .failureCode == "default_artifact_architecture_mismatch"' "$WRONG_REPORT" >/dev/null

SYMLINK_ARTIFACT="$ROOT/symlink-artifact"
ln -s "$DEFAULT_ARTIFACT" "$SYMLINK_ARTIFACT"
SYMLINK_CONFIG="$ROOT/symlink.env"
SYMLINK_REPORT="$ROOT/symlink.json"
write_config "$SYMLINK_CONFIG" "$SYMLINK_ARTIFACT" "$DEFAULT_SHA" "$MCP_ARTIFACT" "$MCP_SHA"
assert_fails run_validator --config "$SYMLINK_CONFIG" --report "$SYMLINK_REPORT" --phase preflight
jq -e '.status == "fail" and .failureCode == "default_artifact_invalid"' "$SYMLINK_REPORT" >/dev/null

HANGING_ARTIFACT="$ROOT/hanging-artifact"
make_hanging_artifact "$HANGING_ARTIFACT"
cat >"$FAKE_BIN/timeout" <<EOF
#!/usr/bin/env bash
set -euo pipefail
[[ "\${1:-}" == -k && "\${2:-}" == 2 && "\${3:-}" == 5 ]]
shift 3
exec '$REAL_TIMEOUT' 0.2 "\$@"
EOF
chmod 700 "$FAKE_BIN/timeout"
HANGING_CONFIG="$ROOT/hanging.env"
HANGING_REPORT="$ROOT/hanging.json"
write_config "$HANGING_CONFIG" "$HANGING_ARTIFACT" "$(sha "$HANGING_ARTIFACT")" "$MCP_ARTIFACT" "$MCP_SHA"
assert_fails run_validator --config "$HANGING_CONFIG" --report "$HANGING_REPORT" --phase preflight
jq -e '.status == "fail" and .failureCode == "default_artifact_version_failed"' "$HANGING_REPORT" >/dev/null
rm -f -- "$FAKE_BIN/timeout"

mkdir -m 700 "$ROOT/report-real" "$ROOT/report-real/nested"
ln -s "$ROOT/report-real" "$ROOT/report-link"
assert_fails run_validator \
  --config "$CONFIG" \
  --report "$ROOT/report-link/nested/evidence.json" \
  --phase preflight
grep -Fq report_parent_not_canonical "$ROOT/last.stderr" || fail_test "noncanonical report parent was not rejected"
assert_no_private_output "noncanonical report-parent output" "$ROOT/last.stdout" "$ROOT/last.stderr"

mkdir -m 755 "$ROOT/public-report-parent"
assert_fails run_validator \
  --config "$CONFIG" \
  --report "$ROOT/public-report-parent/evidence.json" \
  --phase preflight
grep -Fq report_parent_invalid "$ROOT/last.stderr" || fail_test "nonprivate report parent was not rejected"
assert_no_private_output "nonprivate report-parent output" "$ROOT/last.stdout" "$ROOT/last.stderr"

MISSING_CONFIG="$ROOT/missing.env"
MISSING_REPORT="$ROOT/missing.json"
grep -v '^ANDROID_RUN_ID=' "$CONFIG" >"$MISSING_CONFIG"
chmod 600 "$MISSING_CONFIG"
assert_fails run_validator --config "$MISSING_CONFIG" --report "$MISSING_REPORT" --phase preflight
[[ ! -e "$MISSING_REPORT" ]] || fail_test "invalid metadata unexpectedly produced a report"
grep -Fq workflow_metadata_invalid "$ROOT/last.stderr" || fail_test "missing metadata failure code absent"

SUSTAINED_CONFIG="$ROOT/sustained.env"
SUSTAINED_REPORT="$ROOT/sustained.json"
write_config "$SUSTAINED_CONFIG" "$DEFAULT_ARTIFACT" "$DEFAULT_SHA" "$MCP_ARTIFACT" "$MCP_SHA" pass 30 stable
assert_fails run_validator --config "$SUSTAINED_CONFIG" --report "$SUSTAINED_REPORT" --phase preflight
[[ ! -e "$SUSTAINED_REPORT" ]] || fail_test "short sustained window unexpectedly produced a report"
grep -Fq sustained_window_too_short "$ROOT/last.stderr" || fail_test "sustained-window failure code absent"

FAILED_OBSERVATION_CONFIG="$ROOT/failed-observation.env"
FAILED_OBSERVATION_REPORT="$ROOT/failed-observation.json"
write_config "$FAILED_OBSERVATION_CONFIG" "$DEFAULT_ARTIFACT" "$DEFAULT_SHA" "$MCP_ARTIFACT" "$MCP_SHA" fail 15 thermal_limit
assert_fails run_validator --config "$FAILED_OBSERVATION_CONFIG" --report "$FAILED_OBSERVATION_REPORT" --phase preflight
jq -e '
  .status == "fail"
  and .failureCode == "sustained_observation_failed"
  and .releaseEligible == false
  and .phases.preflight == "pass"
  and .sustainedObservation.status == "fail"
  and .sustainedObservation.minutes == 15
  and .sustainedObservation.reasonCode == "thermal_limit"
' "$FAILED_OBSERVATION_REPORT" >/dev/null

RUNTIME_DEFAULT="$ROOT/runtime-default-artifact"
RUNTIME_MCP="$ROOT/runtime-mcp-artifact"
RUNTIME_VOLUME_CONTROL="$ROOT/runtime-volume-control-artifact"
make_runtime_artifact "$RUNTIME_DEFAULT" 0.5.1 default
make_runtime_artifact "$RUNTIME_MCP" 0.5.1 mcp
make_runtime_artifact "$RUNTIME_VOLUME_CONTROL" 0.5.1 volume-control
VOLUME_CONTROL_ARTIFACT="$RUNTIME_VOLUME_CONTROL"
VOLUME_CONTROL_SHA="$(sha "$VOLUME_CONTROL_ARTIFACT")"
RUNTIME_CONFIG="$ROOT/runtime.env"
RUNTIME_REPORT="$ROOT/runtime.json"
write_config "$RUNTIME_CONFIG" "$RUNTIME_DEFAULT" "$(sha "$RUNTIME_DEFAULT")" "$RUNTIME_MCP" "$(sha "$RUNTIME_MCP")"

chmod 640 "$TOKEN_FILE"
TOKEN_MODE_REPORT="$ROOT/token-mode.json"
assert_fails run_validator \
  --config "$RUNTIME_CONFIG" \
  --report "$TOKEN_MODE_REPORT" \
  --phase runtime \
  --confirm-runtime-mutation
jq -e '.status == "fail" and .failureCode == "auth_token_file_invalid" and .phases.runtime == "fail"' "$TOKEN_MODE_REPORT" >/dev/null
chmod 600 "$TOKEN_FILE"

printf '%s\n' 'fixture-private-token' >"$TOKEN_FILE"
TOKEN_NEWLINE_REPORT="$ROOT/token-newline.json"
assert_fails run_validator \
  --config "$RUNTIME_CONFIG" \
  --report "$TOKEN_NEWLINE_REPORT" \
  --phase runtime \
  --confirm-runtime-mutation
jq -e '.status == "fail" and .failureCode == "auth_token_invalid" and .phases.runtime == "fail"' "$TOKEN_NEWLINE_REPORT" >/dev/null
printf '%s' 'fixture-private-token' >"$TOKEN_FILE"
chmod 600 "$TOKEN_FILE"

if ! run_validator \
  --config "$RUNTIME_CONFIG" \
  --report "$RUNTIME_REPORT" \
  --phase runtime \
  --confirm-runtime-mutation >"$ROOT/runtime.stdout" 2>"$ROOT/runtime.stderr"; then
  sed -n '1,240p' "$ROOT/runtime.stdout" >&2
  sed -n '1,240p' "$ROOT/runtime.stderr" >&2
  [[ ! -f "$RUNTIME_REPORT" ]] || jq . "$RUNTIME_REPORT" >&2
  fail_test "valid runtime phase failed"
fi

jq -e '
  .status == "fixture"
  and .phases.preflight == "pass"
  and .phases.runtime == "pass"
  and .phases.deployment == "not_run"
  and .releaseEligible == false
  and ([.results[].code] | index("default_mcp_route_absent") != null)
  and ([.results[].code] | index("authentication_precedes_transport_security") != null)
  and ([.results[].code] | index("disallowed_host_rejected") != null)
  and ([.results[].code] | index("missing_origin_rejected") != null)
  and ([.results[].code] | index("disallowed_origin_rejected") != null)
  and ([.results[].code] | index("exact_tool_allowlist") != null)
  and ([.results[].code] | index("read_only_metadata_verified") != null)
  and ([.results[].code] | index("deterministic_bounded_list") != null)
  and ([.results[].code] | index("safe_root_directory_creation_verified") != null)
  and ([.results[].code] | index("request_scoped_single_use_grant_enforced") != null)
  and ([.results[].code] | index("safe_root_file_copy_verified") != null)
  and ([.results[].code] | index("safe_root_file_hash_verified") != null)
  and ([.results[].code] | index("safe_root_path_metadata_succeeded") != null)
  and ([.results[].code] | index("safe_root_text_search_succeeded") != null)
  and ([.results[].code] | index("read_response_bound_enforced") != null)
  and ([.results[].code] | index("symlink_escape_rejected") != null)
  and ([.results[].code] | index("authentication_precedes_body_limit") != null)
  and ([.results[].code] | index("incompatible_volume_control_artifact_rejected") != null)
  and ([.results[].code] | index("volume_control_posture_verified") != null)
  and ([.results[].code] | index("volume_control_hidden_while_disabled") != null)
  and ([.results[].code] | index("volume_control_runtime_status_read") != null)
  and ([.results[].code] | index("volume_control_disabled_call_rejected") != null)
' "$RUNTIME_REPORT" >/dev/null
if grep -Fq "$ROOT" "$RUNTIME_REPORT" \
  || grep -Fq fixture-private-token "$RUNTIME_REPORT" \
  || grep -Fq fixture-session "$RUNTIME_REPORT" \
  || grep -Fq outside-private-content "$RUNTIME_REPORT"; then
  fail_test "runtime report exposed private validation data"
fi
assert_no_private_output "runtime output" "$ROOT/runtime.stdout" "$ROOT/runtime.stderr"
[[ -z "$(find "$SAFE_ROOT" -mindepth 1 -print -quit)" ]] || fail_test "runtime phase left safe-root state"

python3 -m http.server 18765 --bind 127.0.0.1 >"$ROOT/listener.log" 2>&1 &
LISTENER_PID=$!
for _attempt in $(seq 1 40); do
  (exec 9<>/dev/tcp/127.0.0.1/18765) >/dev/null 2>&1 && break
  sleep 0.05
done
kill -0 "$LISTENER_PID" >/dev/null 2>&1 || fail_test "port-collision fixture did not start"
PORT_COLLISION_REPORT="$ROOT/port-collision.json"
assert_fails run_validator \
  --config "$RUNTIME_CONFIG" \
  --report "$PORT_COLLISION_REPORT" \
  --phase runtime \
  --confirm-runtime-mutation
jq -e '.status == "fail" and .failureCode == "runtime_port_in_use" and .phases.runtime == "fail"' "$PORT_COLLISION_REPORT" >/dev/null
[[ -z "$(find "$SAFE_ROOT" -mindepth 1 -print -quit)" ]] || fail_test "port collision mutated safe root"
kill "$LISTENER_PID" >/dev/null 2>&1 || true
wait "$LISTENER_PID" >/dev/null 2>&1 || true
LISTENER_PID=""

SWAPPED_CONFIG="$ROOT/swapped.env"
SWAPPED_REPORT="$ROOT/swapped.json"
write_config "$SWAPPED_CONFIG" "$RUNTIME_MCP" "$(sha "$RUNTIME_MCP")" "$RUNTIME_DEFAULT" "$(sha "$RUNTIME_DEFAULT")"
assert_fails run_validator \
  --config "$SWAPPED_CONFIG" \
  --report "$SWAPPED_REPORT" \
  --phase runtime \
  --confirm-runtime-mutation
jq -e '.status == "fail" and .failureCode == "default_feature_posture_mismatch" and .phases.runtime == "fail"' "$SWAPPED_REPORT" >/dev/null
[[ -z "$(find "$SAFE_ROOT" -mindepth 1 -print -quit)" ]] || fail_test "failed runtime phase left safe-root state"

RUNTIME_WRONG_VOLUME="$ROOT/runtime-wrong-volume-artifact"
make_runtime_artifact "$RUNTIME_WRONG_VOLUME" 0.5.1 mcp
printf '%s\n' '# distinct incompatible posture fixture' >>"$RUNTIME_WRONG_VOLUME"
chmod 700 "$RUNTIME_WRONG_VOLUME"
VOLUME_CONTROL_ARTIFACT="$RUNTIME_WRONG_VOLUME"
VOLUME_CONTROL_SHA="$(sha "$VOLUME_CONTROL_ARTIFACT")"
WRONG_VOLUME_CONFIG="$ROOT/wrong-volume-runtime.env"
WRONG_VOLUME_REPORT="$ROOT/wrong-volume-runtime.json"
write_config "$WRONG_VOLUME_CONFIG" "$RUNTIME_DEFAULT" "$(sha "$RUNTIME_DEFAULT")" "$RUNTIME_MCP" "$(sha "$RUNTIME_MCP")"
assert_fails run_validator \
  --config "$WRONG_VOLUME_CONFIG" \
  --report "$WRONG_VOLUME_REPORT" \
  --phase runtime \
  --confirm-runtime-mutation
jq -e '.status == "fail" and .failureCode == "volume_control_runtime_status_invalid" and .phases.runtime == "fail"' "$WRONG_VOLUME_REPORT" >/dev/null
[[ -z "$(find "$SAFE_ROOT" -mindepth 1 -print -quit)" ]] || fail_test "wrong control posture left safe-root state"
VOLUME_CONTROL_ARTIFACT="$RUNTIME_VOLUME_CONTROL"
VOLUME_CONTROL_SHA="$(sha "$VOLUME_CONTROL_ARTIFACT")"

NO_RUNTIME_CONFIRM_REPORT="$ROOT/no-runtime-confirm.json"
assert_fails run_validator \
  --config "$RUNTIME_CONFIG" \
  --report "$NO_RUNTIME_CONFIRM_REPORT" \
  --phase runtime
jq -e '.status == "fail" and .failureCode == "runtime_confirmation_missing"' "$NO_RUNTIME_CONFIRM_REPORT" >/dev/null
[[ -z "$(find "$SAFE_ROOT" -mindepth 1 -print -quit)" ]] || fail_test "unconfirmed runtime phase mutated safe root"

DEPLOY_CONFIG="$ROOT/deployment.env"
DEPLOY_REPORT="$ROOT/deployment.json"
write_config "$DEPLOY_CONFIG" "$RUNTIME_DEFAULT" "$(sha "$RUNTIME_DEFAULT")" "$RUNTIME_MCP" "$(sha "$RUNTIME_MCP")"
run_validator \
  --config "$DEPLOY_CONFIG" \
  --report "$DEPLOY_REPORT" \
  --phase deployment \
  --confirm-deployment-mutation >"$ROOT/deployment.stdout" 2>"$ROOT/deployment.stderr"

jq -e '
  .status == "fixture"
  and .phases.preflight == "pass"
  and .phases.runtime == "not_run"
  and .phases.deployment == "pass"
  and ([.results[].code] | index("default_install_baseline_succeeded") != null)
  and ([.results[].code] | index("default_upgrade_candidate_succeeded") != null)
  and ([.results[].code] | index("default_rollback_success_succeeded") != null)
  and ([.results[].code] | index("default_uninstall_success_succeeded") != null)
  and ([.results[].code] | index("install_baseline_succeeded") != null)
  and ([.results[].code] | index("forced_candidate_failure_rejected_and_recovered") != null)
  and ([.results[].code] | index("upgrade_candidate_succeeded") != null)
  and ([.results[].code] | index("rollback_failure_recovery_rejected_and_recovered") != null)
  and ([.results[].code] | index("rollback_success_succeeded") != null)
  and ([.results[].code] | index("uninstall_success_succeeded") != null)
' "$DEPLOY_REPORT" >/dev/null
if grep -Fq "$ROOT" "$DEPLOY_REPORT" || grep -Fq fixture-private-token "$DEPLOY_REPORT"; then
  fail_test "deployment report exposed a path or token"
fi
assert_no_private_output "deployment output" "$ROOT/deployment.stdout" "$ROOT/deployment.stderr"
[[ -z "$(find "$HOME/.local/share" -maxdepth 1 -name 'termux-mcp-release-validation-*' -print -quit 2>/dev/null)" ]] || fail_test "deployment phase left release state"
[[ -z "$(find "$HOME/.config" -maxdepth 1 -name 'termux-mcp-release-validation-*' -print -quit 2>/dev/null)" ]] || fail_test "deployment phase left configuration state"
[[ -z "$(find "$PREFIX/var" -maxdepth 1 -name 'service-termux-mcp-release-validation-*' -print -quit 2>/dev/null)" ]] || fail_test "deployment phase left service state"
[[ -z "$(find "$SAFE_ROOT" -mindepth 1 -print -quit)" ]] || fail_test "deployment phase left safe-root state"

NO_DEPLOY_CONFIRM_REPORT="$ROOT/no-deployment-confirm.json"
assert_fails run_validator \
  --config "$DEPLOY_CONFIG" \
  --report "$NO_DEPLOY_CONFIRM_REPORT" \
  --phase deployment
jq -e '.status == "fail" and .failureCode == "deployment_confirmation_missing"' "$NO_DEPLOY_CONFIRM_REPORT" >/dev/null
[[ -z "$(find "$SAFE_ROOT" -mindepth 1 -print -quit)" ]] || fail_test "unconfirmed deployment phase mutated safe root"

PRODUCTION_CONFIRM_REPORT="$ROOT/production-confirm.json"
if HOME="$HOME" PREFIX="$PREFIX" PATH="$FAKE_BIN:$REAL_PATH" \
  bash "$SCRIPT" \
    --config "$DEPLOY_CONFIG" \
    --report "$PRODUCTION_CONFIRM_REPORT" \
    --phase deployment \
    --confirm-deployment-mutation \
    --production-action uninstall >"$ROOT/production-confirm.stdout" 2>"$ROOT/production-confirm.stderr"; then
  fail_test "production action without exact confirmation unexpectedly succeeded"
fi
jq -e '.status == "fail" and .failureCode == "production_confirmation_invalid"' "$PRODUCTION_CONFIRM_REPORT" >/dev/null
[[ ! -e "$HOME/.local/share/termux-mcp-edge" ]] || fail_test "unconfirmed production action mutated deployment state"

PRODUCTION_ENV_REPORT="$ROOT/production-environment.json"
if HOME="$HOME" PREFIX="$PREFIX" PATH="$FAKE_BIN:$REAL_PATH" \
  bash "$SCRIPT" \
    --config "$DEPLOY_CONFIG" \
    --report "$PRODUCTION_ENV_REPORT" \
    --phase deployment \
    --confirm-deployment-mutation \
    --production-action uninstall \
    --confirm-production-roots termux-mcp-edge-production-uninstall >"$ROOT/production-environment.stdout" 2>"$ROOT/production-environment.stderr"; then
  fail_test "production action outside Termux unexpectedly succeeded"
fi
jq -e '.status == "fail" and .failureCode == "production_termux_environment_required"' "$PRODUCTION_ENV_REPORT" >/dev/null
[[ ! -e "$HOME/.local/share/termux-mcp-edge" ]] || fail_test "non-Termux production action mutated deployment state"
assert_no_private_output "production guard output" "$ROOT/production-environment.stdout" "$ROOT/production-environment.stderr"

INTERRUPT_REPORT="$ROOT/interrupted.json"
if HOME="$HOME" PREFIX="$PREFIX" PATH="$FAKE_BIN:$REAL_PATH" \
  TERMUX_MCP_RELEASE_VALIDATOR_TEST_MODE=1 \
  TERMUX_MCP_RELEASE_VALIDATOR_TEST_INTERRUPT_AFTER_INSTALL=1 \
  bash "$SCRIPT" \
    --config "$DEPLOY_CONFIG" \
    --report "$INTERRUPT_REPORT" \
    --phase deployment \
    --confirm-deployment-mutation >"$ROOT/interrupted.stdout" 2>"$ROOT/interrupted.stderr"; then
  fail_test "interrupted deployment unexpectedly succeeded"
fi
jq -e '.status == "fail" and .failureCode == "interrupted" and .phases.deployment == "running"' "$INTERRUPT_REPORT" >/dev/null
[[ -z "$(find "$HOME/.local/share" -maxdepth 1 -name 'termux-mcp-release-validation-*' -print -quit 2>/dev/null)" ]] || fail_test "interrupted deployment left release state"
[[ -z "$(find "$HOME/.config" -maxdepth 1 -name 'termux-mcp-release-validation-*' -print -quit 2>/dev/null)" ]] || fail_test "interrupted deployment left configuration state"
[[ -z "$(find "$PREFIX/var" -maxdepth 1 -name 'service-termux-mcp-release-validation-*' -print -quit 2>/dev/null)" ]] || fail_test "interrupted deployment left service state"
[[ -z "$(find "$SAFE_ROOT" -mindepth 1 -print -quit)" ]] || fail_test "interrupted deployment left safe-root state"
assert_no_private_output "interrupted output" "$ROOT/interrupted.stdout" "$ROOT/interrupted.stderr"

ALL_CONFIG="$ROOT/all.env"
ALL_REPORT="$ROOT/all.json"
write_config "$ALL_CONFIG" "$RUNTIME_DEFAULT" "$(sha "$RUNTIME_DEFAULT")" "$RUNTIME_MCP" "$(sha "$RUNTIME_MCP")" pass 60 stable
run_validator \
  --config "$ALL_CONFIG" \
  --report "$ALL_REPORT" \
  --phase all \
  --confirm-runtime-mutation \
  --confirm-deployment-mutation >"$ROOT/all.stdout" 2>"$ROOT/all.stderr"
jq -e '
  .status == "fixture"
  and .releaseEligible == false
  and .phases.preflight == "pass"
  and .phases.runtime == "pass"
  and .phases.deployment == "pass"
  and .sustainedObservation.status == "pass"
  and .sustainedObservation.minutes == 60
  and .sustainedObservation.reasonCode == "stable"
' "$ALL_REPORT" >/dev/null
[[ -z "$(find "$SAFE_ROOT" -mindepth 1 -print -quit)" ]] || fail_test "all phase left safe-root state"
[[ -z "$(find "$PREFIX/var" -maxdepth 1 -name 'service-termux-mcp-release-validation-*' -print -quit 2>/dev/null)" ]] || fail_test "all phase left service state"
assert_no_private_output "all-phase output" "$ROOT/all.stdout" "$ROOT/all.stderr"

for evidence_report in \
  "$REPORT" \
  "$BAD_REPORT" \
  "$MANIFEST_REPORT" \
  "$VOLUME_MANIFEST_REPORT" \
  "$MISSING_MANIFEST_REPORT" \
  "$MUTATING_REPORT" \
  "$INJECTION_REPORT" \
  "$WRONG_REPORT" \
  "$SYMLINK_REPORT" \
  "$HANGING_REPORT" \
  "$FAILED_OBSERVATION_REPORT" \
  "$RUNTIME_REPORT" \
  "$TOKEN_MODE_REPORT" \
  "$TOKEN_NEWLINE_REPORT" \
  "$PORT_COLLISION_REPORT" \
  "$SWAPPED_REPORT" \
  "$WRONG_VOLUME_REPORT" \
  "$NO_RUNTIME_CONFIRM_REPORT" \
  "$DEPLOY_REPORT" \
  "$NO_DEPLOY_CONFIRM_REPORT" \
  "$PRODUCTION_CONFIRM_REPORT" \
  "$PRODUCTION_ENV_REPORT" \
  "$INTERRUPT_REPORT" \
  "$ALL_REPORT"; do
  assert_report_contract "$evidence_report"
done

printf 'Termux release validator preflight, runtime, and deployment tests passed\n'
