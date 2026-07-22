#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
SCRIPT="$ROOT/scripts/cross_compile.sh"
CI_WORKFLOW="$ROOT/.github/workflows/ci.yml"
ANDROID_WORKFLOW="$ROOT/.github/workflows/android-cross-compile.yml"
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
assert_contains 'cargo test --locked --workspace --all-targets --all-features' "$CI_WORKFLOW"
[[ "$(grep -Fc 'git diff --exit-code -- Cargo.toml Cargo.lock' "$CI_WORKFLOW")" -eq 4 ]] \
  || fail ci_dependency_input_brackets_changed

metadata_line="$(grep -nF 'cargo metadata --locked --all-features --format-version 1 --no-deps' "$CI_WORKFLOW" | head -n1 | cut -d: -f1)"
cache_line="$(grep -nF 'uses: Swatinem/rust-cache@' "$CI_WORKFLOW" | head -n1 | cut -d: -f1)"
[[ "$metadata_line" =~ ^[1-9][0-9]*$ && "$cache_line" =~ ^[1-9][0-9]*$ ]]
((metadata_line < cache_line)) || fail locked_metadata_must_precede_cargo_aware_cache

assert_contains 'cargo metadata --locked --format-version 1 --no-deps' "$ANDROID_WORKFLOW"
assert_contains 'git diff --exit-code -- Cargo.toml Cargo.lock' "$ANDROID_WORKFLOW"

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
