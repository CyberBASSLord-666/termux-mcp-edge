#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/package_android_artifact.sh"
REAL_PATH="$PATH"

fail_test() {
  printf 'FAIL: %s\n' "$1" >&2
  exit 1
}

assert_fails() {
  if "$@" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail_test "command unexpectedly succeeded"
  fi
}

mkdir -m 700 "$ROOT/fake-bin" "$ROOT/output"
cat >"$ROOT/fake-bin/file" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
target="${*: -1}"
if grep -Fq wrong-arch "$target"; then
  printf '%s\n' 'ELF 64-bit LSB executable, x86-64, for GNU/Linux'
elif grep -Fq linker-only "$target"; then
  printf '%s\n' 'ELF 64-bit LSB pie executable, ARM aarch64, version 1 (SYSV), dynamically linked, interpreter /system/bin/linker64, stripped'
else
  printf '%s\n' 'ELF 64-bit LSB pie executable, ARM aarch64, for Android 24'
fi
EOF
chmod 700 "$ROOT/fake-bin/file"

make_binary() {
  local path="$1"
  printf '%s\n' '#!/usr/bin/env bash' 'exit 0' >"$path"
  chmod 700 "$path"
}

run_package() {
  PATH="$ROOT/fake-bin:$REAL_PATH" bash "$SCRIPT" \
    --binary "$1" \
    --output-dir "$2" \
    --repository CyberBASSLord-666/termux-mcp-edge \
    --commit aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa \
    --workflow-run-id 1003 \
    --artifact-name "$3" \
    --posture "$4" \
    --version 0.5.1
}

bash -n "$SCRIPT"
BINARY="$ROOT/candidate"
BUNDLE="$ROOT/output/default"
make_binary "$BINARY"
run_package "$BINARY" "$BUNDLE" termux-mcp-server-aarch64-linux-android-default default >"$ROOT/package.stdout"
[[ -x "$BUNDLE/termux-mcp-server" ]] || fail_test "packaged binary is not executable"
[[ "$(stat -c '%a' "$BUNDLE")" == 700 ]] || fail_test "bundle mode is not 700"
[[ "$(stat -c '%a' "$BUNDLE/termux-mcp-server")" == 700 ]] || fail_test "binary mode is not 700"
[[ "$(stat -c '%a' "$BUNDLE/SHA256SUMS")" == 600 ]] || fail_test "checksum mode is not 600"
[[ "$(stat -c '%a' "$BUNDLE/artifact-manifest.json")" == 600 ]] || fail_test "manifest mode is not 600"
(cd "$BUNDLE" && sha256sum -c SHA256SUMS >/dev/null)
jq -e '
  (keys == ["artifactName","bytes","commit","createdAt","elf","features","fileName","posture","repository","schemaVersion","sha256","target","version","workflowRunId"])
  and .schemaVersion == 1
  and .repository == "CyberBASSLord-666/termux-mcp-edge"
  and .commit == "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  and .workflowRunId == "1003"
  and .artifactName == "termux-mcp-server-aarch64-linux-android-default"
  and .posture == "default"
  and .features == []
  and .target == "aarch64-linux-android"
  and .fileName == "termux-mcp-server"
  and .version == "0.5.1"
  and .elf == "aarch64-android-elf"
' "$BUNDLE/artifact-manifest.json" >/dev/null

MCP_BUNDLE="$ROOT/output/mcp"
run_package "$BINARY" "$MCP_BUNDLE" termux-mcp-server-aarch64-linux-android-mcp-runtime mcp-runtime >/dev/null
jq -e '.posture == "mcp-runtime" and .features == ["mcp-runtime"]' "$MCP_BUNDLE/artifact-manifest.json" >/dev/null

LINKER_ONLY="$ROOT/linker-only-candidate"
printf '%s\n' '#!/usr/bin/env bash' '# linker-only' 'exit 0' >"$LINKER_ONLY"
chmod 700 "$LINKER_ONLY"
run_package "$LINKER_ONLY" "$ROOT/output/linker-only" termux-mcp-server-aarch64-linux-android-default default >/dev/null
jq -e '.elf == "aarch64-android-elf"' "$ROOT/output/linker-only/artifact-manifest.json" >/dev/null

assert_fails run_package "$BINARY" "$ROOT/output/mismatch" termux-mcp-server-aarch64-linux-android-default mcp-runtime
grep -Fq artifact_name_posture_mismatch "$ROOT/last.stderr" || fail_test "posture mismatch code absent"
[[ ! -e "$ROOT/output/mismatch" ]] || fail_test "posture mismatch created output"

WRONG_ARCH="$ROOT/wrong-arch-candidate"
printf '%s\n' '#!/usr/bin/env bash' '# wrong-arch' 'exit 0' >"$WRONG_ARCH"
chmod 700 "$WRONG_ARCH"
assert_fails run_package "$WRONG_ARCH" "$ROOT/output/wrong-arch" termux-mcp-server-aarch64-linux-android-default default
grep -Fq binary_architecture_mismatch "$ROOT/last.stderr" || fail_test "wrong architecture code absent"

SYMLINK="$ROOT/symlink-candidate"
ln -s "$BINARY" "$SYMLINK"
assert_fails run_package "$SYMLINK" "$ROOT/output/symlink" termux-mcp-server-aarch64-linux-android-default default
grep -Fq binary_invalid "$ROOT/last.stderr" || fail_test "symlink code absent"

mkdir "$ROOT/output/existing"
assert_fails run_package "$BINARY" "$ROOT/output/existing" termux-mcp-server-aarch64-linux-android-default default
grep -Fq output_directory_invalid "$ROOT/last.stderr" || fail_test "existing output code absent"
[[ -z "$(find "$ROOT/output" -maxdepth 1 -name '*.staging.*' -print -quit)" ]] || fail_test "failed packaging left staging state"

printf 'Android artifact packaging tests passed\n'
