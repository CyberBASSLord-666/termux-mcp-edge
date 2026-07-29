#!/usr/bin/env bash
set -Eeuo pipefail
IFS=$'\n\t'

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
HELPER="$REPO_ROOT/scripts/commit_verified_file.py"
REAL_PYTHON3="$(command -v python3)"
REAL_MV="$(command -v mv)"
REAL_LN="$(command -v ln)"

fail() {
  printf 'verified file commit test failed: %s\n' "$1" >&2
  exit 1
}

expect_failure() {
  if "$@" >"$ROOT/last.stdout" 2>"$ROOT/last.stderr"; then
    fail command_unexpectedly_succeeded
  fi
}

[[ -f "$HELPER" && ! -L "$HELPER" ]] || fail helper_missing_or_linked
python3 - "$HELPER" <<'PY'
import pathlib
import sys

source = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
compile(source, sys.argv[1], "exec")
PY

mkdir -m 700 "$ROOT/happy"
printf 'verified evidence\n' >"$ROOT/happy/source"
chmod 600 "$ROOT/happy/source"
source_sha="$(sha256sum "$ROOT/happy/source" | awk '{print $1}')"
python3 "$HELPER" \
  --source "$ROOT/happy/source" \
  --destination "$ROOT/happy/output" \
  --sha256 "$source_sha" \
  --mode 600
[[ -f "$ROOT/happy/output" && ! -L "$ROOT/happy/output" ]] \
  || fail output_missing_or_linked
[[ "$(stat -Lc '%d:%i' "$ROOT/happy/source")" == "$(stat -Lc '%d:%i' "$ROOT/happy/output")" ]] \
  || fail output_does_not_reference_verified_inode

mkdir -m 700 "$ROOT/conflict"
printf 'candidate\n' >"$ROOT/conflict/source"
printf 'concurrent owner\n' >"$ROOT/conflict/output"
chmod 600 "$ROOT/conflict/source" "$ROOT/conflict/output"
source_sha="$(sha256sum "$ROOT/conflict/source" | awk '{print $1}')"
expect_failure python3 "$HELPER" \
  --source "$ROOT/conflict/source" \
  --destination "$ROOT/conflict/output" \
  --sha256 "$source_sha" \
  --mode 600
[[ "$(<"$ROOT/conflict/output")" == "concurrent owner" ]] \
  || fail destination_conflict_was_changed

mkdir -m 700 "$ROOT/hard-link-conflict"
printf 'candidate\n' >"$ROOT/hard-link-conflict/source"
chmod 600 "$ROOT/hard-link-conflict/source"
ln "$ROOT/hard-link-conflict/source" "$ROOT/hard-link-conflict/output"
source_sha="$(sha256sum "$ROOT/hard-link-conflict/source" | awk '{print $1}')"
expect_failure python3 "$HELPER" \
  --source "$ROOT/hard-link-conflict/source" \
  --destination "$ROOT/hard-link-conflict/output" \
  --sha256 "$source_sha" \
  --mode 600
[[ "$(stat -Lc '%d:%i' "$ROOT/hard-link-conflict/source")" == "$(stat -Lc '%d:%i' "$ROOT/hard-link-conflict/output")" ]] \
  || fail hard_link_conflict_was_removed_or_replaced

mkdir -m 700 "$ROOT/source-race" "$ROOT/fake-python"
printf 'validated\n' >"$ROOT/source-race/source"
printf 'replacement\n' >"$ROOT/source-race/replacement"
chmod 600 "$ROOT/source-race/source" "$ROOT/source-race/replacement"
source_sha="$(sha256sum "$ROOT/source-race/source" | awk '{print $1}')"
cat >"$ROOT/fake-python/python3" <<'EOF'
#!/usr/bin/env bash
set -Eeuo pipefail
source_path=''
previous=''
for argument in "$@"; do
  if [[ "$previous" == --source ]]; then
    source_path="$argument"
    break
  fi
  previous="$argument"
done
"$COMMIT_TEST_REAL_MV" -- "$source_path" "$source_path.validated"
"$COMMIT_TEST_REAL_LN" -s -- "$COMMIT_TEST_REPLACEMENT" "$source_path"
exec "$COMMIT_TEST_REAL_PYTHON3" "$@"
EOF
chmod 700 "$ROOT/fake-python/python3"
expect_failure env \
  PATH="$ROOT/fake-python:$PATH" \
  COMMIT_TEST_REAL_MV="$REAL_MV" \
  COMMIT_TEST_REAL_LN="$REAL_LN" \
  COMMIT_TEST_REPLACEMENT="$ROOT/source-race/replacement" \
  COMMIT_TEST_REAL_PYTHON3="$REAL_PYTHON3" \
  python3 "$HELPER" \
    --source "$ROOT/source-race/source" \
    --destination "$ROOT/source-race/output" \
    --sha256 "$source_sha" \
    --mode 600
[[ ! -e "$ROOT/source-race/output" && ! -L "$ROOT/source-race/output" ]] \
  || fail source_substitution_was_published

mkdir -m 700 "$ROOT/public-parent"
printf 'validated\n' >"$ROOT/public-parent/source"
chmod 600 "$ROOT/public-parent/source"
source_sha="$(sha256sum "$ROOT/public-parent/source" | awk '{print $1}')"
chmod 755 "$ROOT/public-parent"
expect_failure python3 "$HELPER" \
  --source "$ROOT/public-parent/source" \
  --destination "$ROOT/public-parent/output" \
  --sha256 "$source_sha" \
  --mode 600
[[ ! -e "$ROOT/public-parent/output" && ! -L "$ROOT/public-parent/output" ]] \
  || fail nonprivate_parent_received_output

printf 'Verified file no-replace commit tests passed\n'
