#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/setup_named_tunnel.sh"
FAKE_BIN="$ROOT/bin"
CALL_LOG="$ROOT/cloudflared.calls"
STATE_ROOT="$ROOT/state"
mkdir -p "$FAKE_BIN" "$STATE_ROOT"

fail_test() { printf 'assertion failed: %s\n' "$*" >&2; exit 1; }
assert_fails() { if "$@" >"$ROOT/stdout" 2>"$ROOT/stderr"; then fail_test "command unexpectedly succeeded: $*"; fi; }
assert_contains() { grep -F -- "$2" "$1" >/dev/null || fail_test "$1 did not contain: $2"; }
assert_no_calls() { [[ ! -s "$CALL_LOG" ]] || fail_test "cloudflared was unexpectedly called"; }
reset_case() { : >"$CALL_LOG"; rm -rf -- "$STATE_ROOT"; mkdir -p "$STATE_ROOT"; }
assert_temp_clean() { [[ -z "$(find "$ROOT" -maxdepth 1 -name 'termux-mcp-tunnel.*' -print -quit)" ]] || fail_test "private temporary directory leaked"; }

cat >"$FAKE_BIN/cloudflared" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >>"$FAKE_CALL_LOG"
case "$*" in
  'tunnel list --output json')
    [[ "${FAKE_LIST_FAILURE:-0}" == 0 ]] || exit 20
    if [[ -e "$FAKE_STATE_ROOT/created" ]]; then
      printf '[{"name":"termux-mcp"}]\n'
    else
      printf '%s\n' "${FAKE_LIST_JSON:-[]}" 
    fi
    ;;
  'tunnel create termux-mcp')
    [[ "${FAKE_CREATE_FAILURE:-0}" == 0 ]] || exit 22
    touch "$FAKE_STATE_ROOT/created"
    ;;
  'tunnel route dns termux-mcp mcp.example.com')
    [[ "${FAKE_ROUTE_FAILURE:-0}" == 0 ]] || exit 23
    ;;
  *) exit 99 ;;
esac
EOF
chmod 700 "$FAKE_BIN/cloudflared"

export PATH="$FAKE_BIN:$PATH" FAKE_CALL_LOG="$CALL_LOG" FAKE_STATE_ROOT="$STATE_ROOT" TMPDIR="$ROOT"

bash -n "$SCRIPT"

reset_case
assert_fails bash "$SCRIPT"
assert_contains "$ROOT/stderr" "explicit tunnel name and hostname are required"
assert_no_calls

for invalid_hostname in \
  'mcp.yourdomain.com:443' 'https://mcp.example.com' '*.example.com' \
  'mcp..example.com' '-mcp.example.com' 'mcp-.example.com' 'localhost'; do
  reset_case
  assert_fails bash "$SCRIPT" --dry-run termux-mcp "$invalid_hostname"
  assert_no_calls
done

for invalid_tunnel in '-termux' 'termux.mcp' 'termux mcp' 'termux/mcp'; do
  reset_case
  assert_fails bash "$SCRIPT" --dry-run "$invalid_tunnel" mcp.example.com
  assert_no_calls
done

reset_case
bash "$SCRIPT" --dry-run --create termux-mcp mcp.example.com >"$ROOT/stdout"
assert_contains "$ROOT/stdout" "authenticated inventory confirms it is absent"
assert_no_calls

reset_case
FAKE_LIST_FAILURE=1 assert_fails bash "$SCRIPT" termux-mcp mcp.example.com
assert_contains "$ROOT/stderr" "unable to list tunnels"
assert_contains "$CALL_LOG" "tunnel list --output json"
[[ "$(wc -l <"$CALL_LOG")" == 1 ]] || fail_test "list failure triggered mutation"
assert_temp_clean

reset_case
FAKE_LIST_JSON='[]' assert_fails bash "$SCRIPT" termux-mcp mcp.example.com
assert_contains "$ROOT/stderr" "rerun with --create"
[[ "$(wc -l <"$CALL_LOG")" == 1 ]] || fail_test "absent tunnel mutated without --create"
assert_temp_clean

reset_case
FAKE_LIST_JSON='not-json' assert_fails bash "$SCRIPT" termux-mcp mcp.example.com
assert_contains "$ROOT/stderr" "unsupported tunnel-list JSON shape"
assert_temp_clean

reset_case
FAKE_LIST_JSON='[{"name":"termux-mcp"}]' bash "$SCRIPT" termux-mcp mcp.example.com >"$ROOT/stdout"
assert_contains "$ROOT/stdout" "exact tunnel already exists"
assert_contains "$CALL_LOG" "tunnel route dns termux-mcp mcp.example.com"
! grep -F -- '--overwrite-dns' "$CALL_LOG" >/dev/null || fail_test "DNS overwrite was enabled"
assert_temp_clean

reset_case
FAKE_LIST_JSON='[{"name":"termux-mcp"}]' FAKE_ROUTE_FAILURE=1 assert_fails bash "$SCRIPT" termux-mcp mcp.example.com
assert_contains "$ROOT/stderr" "no overwrite attempted"
! grep -F -- '--overwrite-dns' "$CALL_LOG" >/dev/null || fail_test "conflicting DNS route was overwritten"
assert_temp_clean

reset_case
FAKE_LIST_JSON='[]' FAKE_CREATE_FAILURE=1 assert_fails bash "$SCRIPT" --create termux-mcp mcp.example.com
assert_contains "$ROOT/stderr" "tunnel creation failed"
assert_contains "$CALL_LOG" "tunnel create termux-mcp"
! grep -F -- 'tunnel route dns' "$CALL_LOG" >/dev/null || fail_test "route attempted after create failure"
assert_temp_clean

reset_case
FAKE_LIST_JSON='[]' bash "$SCRIPT" --create termux-mcp mcp.example.com >"$ROOT/stdout"
assert_contains "$ROOT/stdout" "tunnel created and confirmed"
assert_contains "$CALL_LOG" "tunnel route dns termux-mcp mcp.example.com"
[[ "$(grep -Fc -- 'tunnel list --output json' "$CALL_LOG")" == 2 ]] || fail_test "created tunnel was not re-confirmed"
assert_temp_clean

printf 'named tunnel setup tests passed\n'
