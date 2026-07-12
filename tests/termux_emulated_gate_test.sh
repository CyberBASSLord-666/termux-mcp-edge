#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GATE="$ROOT/scripts/termux_emulated_gate.sh"
INHERITANCE="$ROOT/scripts/verify_observation_inheritance.sh"
ANDROID_WORKFLOW="$ROOT/.github/workflows/android-cross-compile.yml"
SOURCE_REPORT="$ROOT/docs/release-evidence/v0.5.1-physical-fe5f7b80.json"

fail_test() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

for script in "$GATE" "$INHERITANCE"; do
  bash -n "$script"
  bash "$script" --help | grep -Fq 'Usage:' || fail_test "help output missing for $(basename "$script")"
done

if bash "$GATE" >"$ROOT/.termux-emulated-test.stdout" 2>"$ROOT/.termux-emulated-test.stderr"; then
  fail_test 'gate without required arguments unexpectedly succeeded'
fi
grep -Fq 'reason=expected_commit_invalid' "$ROOT/.termux-emulated-test.stderr" || fail_test 'gate missing deterministic argument failure'

if bash "$INHERITANCE" >"$ROOT/.termux-inheritance-test.stdout" 2>"$ROOT/.termux-inheritance-test.stderr"; then
  fail_test 'inheritance verifier without required arguments unexpectedly succeeded'
fi
grep -Fq 'reason=commit_invalid' "$ROOT/.termux-inheritance-test.stderr" || fail_test 'inheritance verifier missing deterministic argument failure'

rm -f -- \
  "$ROOT/.termux-emulated-test.stdout" "$ROOT/.termux-emulated-test.stderr" \
  "$ROOT/.termux-inheritance-test.stdout" "$ROOT/.termux-inheritance-test.stderr"

jq -e '
  .properties.status.const == "pass"
  and .properties.environment.properties.executionMode.const == "official-termux-docker-native-arm64"
  and .properties.environment.properties.androidLinker.const == true
  and .properties.stress.properties.samples.minimum == 32
  and .properties.stress.properties.highImpactDisabled.const == true
' "$ROOT/docs/emulated-release-evidence-schema-v1.json" >/dev/null

jq -e '
  .properties.releaseQualificationEligible.const == true
  and .properties.evidenceMode.const == "inherited_physical_observation"
  and .properties.sourceObservation.properties.physicalDevice.const == true
  and .properties.sourceObservation.properties.minutes.minimum == 60
  and .properties.equivalence.properties.runtimeSourceUnchanged.const == true
  and .properties.equivalence.properties.candidateArtifactsMatchBridge.const == true
' "$ROOT/docs/release-observation-inheritance-schema-v1.json" >/dev/null

test "$(sha256sum "$SOURCE_REPORT" | awk '{print $1}')" = 677796015065eb193ac78b2dd200de64efccb95a226837a4545c85021cb9283c

grep -Fq 'runs-on: ubuntu-24.04-arm' "$ANDROID_WORKFLOW" || fail_test 'native ARM64 runner missing'
grep -Fq 'termux/termux-docker:aarch64@sha256:926e5c08aebc6df89f1cb3d9558c3b56b6246e59305fcd707bdf68f2584493b3' "$ANDROID_WORKFLOW" || fail_test 'pinned official Termux image missing'
grep -Fq 'uses: actions/download-artifact@70fc10c6e5e1ce46ad2ea6f2b72d43f7d47b13c3' "$ANDROID_WORKFLOW" || fail_test 'download action is not pinned'

chmod_line="$(grep -nF "chmod 700 \"\$output_root\"" "$ANDROID_WORKFLOW" | cut -d: -f1)"
chown_line="$(grep -nF "sudo chown 1000:1000 \"\$output_root\"" "$ANDROID_WORKFLOW" | cut -d: -f1)"
[[ "$chmod_line" =~ ^[0-9]+$ && "$chown_line" =~ ^[0-9]+$ ]] || fail_test 'private output ownership sequence missing'
((chmod_line < chown_line)) || fail_test 'output mode must be set before ownership transfers to the container user'

printf 'Native ARM64 Termux gate and observation inheritance contract tests passed\n'
