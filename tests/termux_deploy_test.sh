#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/termux_deploy.sh"

fail_test() { printf 'assertion failed: %s\n' "$*" >&2; exit 1; }
assert_eq() { [[ "$1" == "$2" ]] || fail_test "expected $2, got $1"; }
assert_fails() { if "$@" >/dev/null 2>&1; then fail_test "command unexpectedly succeeded: $*"; fi; }

make_shell() {
  local prefix="$1"
  mkdir -p "$prefix/bin"
  # A shebang interpreter must be a native executable. Copy the host POSIX
  # shell to model Termux's binary $PREFIX/bin/sh; a script wrapper cannot
  # itself reliably serve as a Linux shebang interpreter.
  cp -L -- /bin/sh "$prefix/bin/sh"
  chmod 700 "$prefix/bin/sh"
}
make_config() {
  local config_root="$1"
  mkdir -p "$config_root"; chmod 700 "$config_root"
  cat >"$config_root/runtime.env" <<'EOF'
MCP__AUTH__STATIC_TOKEN=test-static-token
MCP__SERVER__HOST=127.0.0.1
MCP__SERVER__PORT=8000
MCP__TRANSPORT__ALLOWED_HOSTS=localhost:8000,127.0.0.1:8000
MCP__TRANSPORT__ALLOWED_ORIGINS=http://localhost:8000,http://127.0.0.1:8000
MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true
MCP__CAPABILITY__KEY_ID=deployment-test-1
MCP__CAPABILITY__HMAC_KEY_HEX=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
RUST_LOG=termux_mcp_server=info
EOF
  chmod 600 "$config_root/runtime.env"
}
make_artifact() {
  local path="$1" version="$2"
  cat >"$path" <<EOF
#!/bin/sh
if [ "\${1:-}" = "--version" ]; then printf 'termux-mcp-server %s\\n' '$version'; exit 0; fi
exit 0
EOF
  chmod 700 "$path"
}
artifact_sha() { sha256sum -- "$1" | awk '{print $1}'; }
configure_environment() {
  local root="$1"
  export HOME="$root/home" PREFIX="$root/prefix"
  export TERMUX_MCP_DEPLOY_ROOT="$HOME/.local/share/termux-mcp-edge"
  export TERMUX_MCP_CONFIG_ROOT="$HOME/.config/termux-mcp-edge"
  export TERMUX_MCP_SERVICE_ROOT="$PREFIX/var/service"
  export TERMUX_MCP_SERVICE_SHELL="$PREFIX/bin/sh"
  export TERMUX_MCP_TEST_MODE=1 TERMUX_MCP_TEST_PROBE_SEQUENCE=success TERMUX_MCP_TEST_STOP_SEQUENCE=success TERMUX_MCP_TEST_START_SEQUENCE=success
  unset TERMUX_MCP_ALLOW_UNVERIFIED_ARTIFACT TERMUX_MCP_DRY_RUN
  mkdir -p "$HOME" "$PREFIX"; make_shell "$PREFIX"; make_config "$TERMUX_MCP_CONFIG_ROOT"
}

bash -n "$SCRIPT"
configure_environment "$ROOT/main"
ARTIFACT_100="$ROOT/server-1.0.0"; ARTIFACT_110="$ROOT/server-1.1.0"; ARTIFACT_120="$ROOT/server-1.2.0"; ARTIFACT_200="$ROOT/server-2.0.0"
make_artifact "$ARTIFACT_100" 1.0.0; make_artifact "$ARTIFACT_110" 1.1.0; make_artifact "$ARTIFACT_120" 1.2.0; make_artifact "$ARTIFACT_200" 2.0.0
SHA_100="$(artifact_sha "$ARTIFACT_100")"; SHA_110="$(artifact_sha "$ARTIFACT_110")"; SHA_120="$(artifact_sha "$ARTIFACT_120")"; SHA_200="$(artifact_sha "$ARTIFACT_200")"
BAD_SHA="0000000000000000000000000000000000000000000000000000000000000000"
SERVICE_DIR="$TERMUX_MCP_SERVICE_ROOT/mcp_runtime"

bash "$SCRIPT" install --artifact "$ARTIFACT_100" --version 1.0.0 --sha256 "$SHA_100"
[[ -x "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0/termux-mcp-server" ]]
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"
[[ -x "$SERVICE_DIR/run" && ! -e "$SERVICE_DIR/down" ]]
[[ "$(stat -c '%a' "$SERVICE_DIR")" == 700 && "$(stat -c '%a' "$SERVICE_DIR/run")" == 700 ]]
[[ -z "$(find "$SERVICE_DIR" -maxdepth 1 -name '.run.*' -print -quit)" ]]
[[ ! -e "$SERVICE_DIR/log" ]]
[[ "$(stat -c '%a' "$TERMUX_MCP_CONFIG_ROOT")" == 700 && "$(stat -c '%a' "$TERMUX_MCP_CONFIG_ROOT/runtime.env")" == 600 ]]
head -n 1 "$SERVICE_DIR/run" | grep -Fx "#!$PREFIX/bin/sh"

PWNED="$ROOT/config-was-executed"
printf 'RUST_BACKTRACE=$(touch %s)\n' "$PWNED" >>"$TERMUX_MCP_CONFIG_ROOT/runtime.env"
"$SERVICE_DIR/run"
[[ ! -e "$PWNED" ]]
printf '%s\n' 'MCP__AUTH__STATIC_TOKEN=duplicate-runtime-token' >>"$TERMUX_MCP_CONFIG_ROOT/runtime.env"
assert_fails "$SERVICE_DIR/run"
sed -i '$d' "$TERMUX_MCP_CONFIG_ROOT/runtime.env"

assert_fails bash "$SCRIPT" install --artifact "$ARTIFACT_100" --version 1.0.0 --sha256 "$SHA_100"
TERMUX_MCP_START_ATTEMPTS=0 assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_110" --version 1.1.0 --sha256 "$SHA_110"
assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_110" --version 1.1.0
assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_110" --version 1.1.0 --sha256 "$BAD_SHA"
assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_110" --version 9.9.9 --sha256 "$SHA_110"

ln -s "$ROOT/missing-down-target" "$SERVICE_DIR/down"
assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_110" --version 1.1.0 --sha256 "$SHA_110"
[[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0" ]]
rm -f "$SERVICE_DIR/down"

run_before_sha="$(sha256sum "$SERVICE_DIR/run" | awk '{print $1}')"
current_before="$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")"
TERMUX_MCP_TEST_STOP_SEQUENCE=failure assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_110" --version 1.1.0 --sha256 "$SHA_110"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$current_before"
assert_eq "$(sha256sum "$SERVICE_DIR/run" | awk '{print $1}')" "$run_before_sha"
[[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0" && ! -e "$SERVICE_DIR/down" ]]

bash "$SCRIPT" upgrade --artifact "$ARTIFACT_110" --version 1.1.0 --sha256 "$SHA_110"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"
[[ ! -e "$SERVICE_DIR/down" && -z "$(find "$SERVICE_DIR" -maxdepth 1 -name '.run.*' -print -quit)" ]]

start_failure_run_sha="$(sha256sum "$SERVICE_DIR/run" | awk '{print $1}')"
if TERMUX_MCP_TEST_START_SEQUENCE=failure,success bash "$SCRIPT" upgrade --artifact "$ARTIFACT_120" --version 1.2.0 --sha256 "$SHA_120" >/dev/null 2>&1; then fail_test "start-failing upgrade unexpectedly succeeded"; fi
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"
assert_eq "$(sha256sum "$SERVICE_DIR/run" | awk '{print $1}')" "$start_failure_run_sha"
[[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/releases/1.2.0" && ! -e "$SERVICE_DIR/down" ]]

running_run_sha="$(sha256sum "$SERVICE_DIR/run" | awk '{print $1}')"
if TERMUX_MCP_TEST_PROBE_SEQUENCE=failure,success bash "$SCRIPT" upgrade --artifact "$ARTIFACT_120" --version 1.2.0 --sha256 "$SHA_120" >/dev/null 2>&1; then fail_test "unhealthy running-service upgrade unexpectedly succeeded"; fi
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"
assert_eq "$(sha256sum "$SERVICE_DIR/run" | awk '{print $1}')" "$running_run_sha"
[[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/releases/1.2.0" && ! -e "$SERVICE_DIR/down" ]]

touch "$SERVICE_DIR/down"; chmod 600 "$SERVICE_DIR/down"
stopped_run_sha="$(sha256sum "$SERVICE_DIR/run" | awk '{print $1}')"
if TERMUX_MCP_TEST_PROBE_SEQUENCE=failure bash "$SCRIPT" upgrade --artifact "$ARTIFACT_200" --version 2.0.0 --sha256 "$SHA_200" >/dev/null 2>&1; then fail_test "unhealthy stopped-service upgrade unexpectedly succeeded"; fi
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"
assert_eq "$(sha256sum "$SERVICE_DIR/run" | awk '{print $1}')" "$stopped_run_sha"
[[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/releases/2.0.0" && -f "$SERVICE_DIR/down" ]]
[[ "$(stat -c '%a' "$SERVICE_DIR/down")" == 600 ]]
rm -f "$SERVICE_DIR/down"

if TERMUX_MCP_TEST_PROBE_SEQUENCE=failure,success bash "$SCRIPT" rollback >/dev/null 2>&1; then fail_test "unhealthy rollback unexpectedly succeeded"; fi
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"
[[ ! -e "$SERVICE_DIR/down" ]]

bash "$SCRIPT" rollback
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"

LOCK_DIR="${TERMUX_MCP_DEPLOY_ROOT}.deploy-lock"
mkdir -p "$LOCK_DIR"; printf '%s\n' "$$" >"$LOCK_DIR/owner.pid"
assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_120" --version 1.2.0 --sha256 "$SHA_120"
rm -rf "$LOCK_DIR"; mkdir -p "$LOCK_DIR"; printf '%s\n' 999999 >"$LOCK_DIR/owner.pid"
bash "$SCRIPT" upgrade --artifact "$ARTIFACT_120" --version 1.2.0 --sha256 "$SHA_120"
[[ ! -e "$LOCK_DIR" ]]
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.2.0"

current_before="$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")"
TERMUX_MCP_TEST_STOP_SEQUENCE=failure assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_200" --version 2.0.0 --sha256 "$SHA_200" --dry-run
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$current_before"
[[ -z "$(find "$TERMUX_MCP_DEPLOY_ROOT" -maxdepth 1 -name '.service-snapshot-*' -print -quit)" ]]
bash "$SCRIPT" upgrade --artifact "$ARTIFACT_200" --version 2.0.0 --sha256 "$SHA_200" --dry-run >/dev/null
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$current_before"
[[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/releases/2.0.0" ]]

fake_bin="$ROOT/production-dry-run-bin"; mkdir -p "$fake_bin"
dry_run_external_call="$ROOT/production-dry-run-external-call"
for command_name in sv curl; do
  cat >"$fake_bin/$command_name" <<EOF
#!/bin/sh
touch '$dry_run_external_call'
exit 99
EOF
  chmod 700 "$fake_bin/$command_name"
done
previous_before="$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")"
service_before_sha="$(sha256sum "$SERVICE_DIR/run" | awk '{print $1}')"
TERMUX_MCP_TEST_MODE=0 PATH="$fake_bin:$PATH" bash "$SCRIPT" rollback --dry-run >/dev/null
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$current_before"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$previous_before"
assert_eq "$(sha256sum "$SERVICE_DIR/run" | awk '{print $1}')" "$service_before_sha"
[[ ! -e "$dry_run_external_call" ]]

status="$(bash "$SCRIPT" status)"
[[ "$status" == *"current=$TERMUX_MCP_DEPLOY_ROOT/releases/1.2.0"* && "$status" != *test-static-token* ]]

previous_target="$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")"
rm -f "$TERMUX_MCP_DEPLOY_ROOT/previous"; ln -s /tmp "$TERMUX_MCP_DEPLOY_ROOT/previous"
assert_fails bash "$SCRIPT" rollback
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.2.0"
rm -f "$TERMUX_MCP_DEPLOY_ROOT/previous"; ln -s "$previous_target" "$TERMUX_MCP_DEPLOY_ROOT/previous"

chmod 644 "$TERMUX_MCP_CONFIG_ROOT/runtime.env"; assert_fails bash "$SCRIPT" rollback; chmod 600 "$TERMUX_MCP_CONFIG_ROOT/runtime.env"
printf 'PATH=/tmp\n' >>"$TERMUX_MCP_CONFIG_ROOT/runtime.env"; assert_fails bash "$SCRIPT" rollback; sed -i '/^PATH=/d' "$TERMUX_MCP_CONFIG_ROOT/runtime.env"
sed -i 's/^MCP__SERVER__PORT=8000$/MCP__SERVER__PORT=0/' "$TERMUX_MCP_CONFIG_ROOT/runtime.env"; assert_fails bash "$SCRIPT" rollback; sed -i 's/^MCP__SERVER__PORT=0$/MCP__SERVER__PORT=8000/' "$TERMUX_MCP_CONFIG_ROOT/runtime.env"
printf '%s\n' 'MCP__CAPABILITY__KEY_ID=duplicate-key' >>"$TERMUX_MCP_CONFIG_ROOT/runtime.env"; assert_fails bash "$SCRIPT" rollback; sed -i '$d' "$TERMUX_MCP_CONFIG_ROOT/runtime.env"
sed -i 's/^MCP__CAPABILITY__HMAC_KEY_HEX=.*$/MCP__CAPABILITY__HMAC_KEY_HEX=ABCDEF/' "$TERMUX_MCP_CONFIG_ROOT/runtime.env"; assert_fails bash "$SCRIPT" rollback; sed -i 's/^MCP__CAPABILITY__HMAC_KEY_HEX=ABCDEF$/MCP__CAPABILITY__HMAC_KEY_HEX=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef/' "$TERMUX_MCP_CONFIG_ROOT/runtime.env"
sed -i '/^MCP__CAPABILITY__HMAC_KEY_HEX=/d' "$TERMUX_MCP_CONFIG_ROOT/runtime.env"; assert_fails bash "$SCRIPT" rollback; printf '%s\n' 'MCP__CAPABILITY__HMAC_KEY_HEX=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' >>"$TERMUX_MCP_CONFIG_ROOT/runtime.env"
assert_fails env TERMUX_MCP_DEPLOY_ROOT="$HOME" bash "$SCRIPT" status
assert_fails env TERMUX_MCP_CONFIG_ROOT="$HOME/bad path" bash "$SCRIPT" status
assert_fails env TERMUX_MCP_SERVICE_ROOT="$ROOT/outside-prefix" bash "$SCRIPT" status
assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_200" --version '../bad' --sha256 "$SHA_200"

(
  configure_environment "$ROOT/initial-dry-run"
  artifact="$ROOT/initial-dry-run-server"; make_artifact "$artifact" 3.0.0; sha="$(artifact_sha "$artifact")"
  bash "$SCRIPT" install --artifact "$artifact" --version 3.0.0 --sha256 "$sha" --dry-run >/dev/null
  [[ ! -e "$TERMUX_MCP_DEPLOY_ROOT" && ! -e "$TERMUX_MCP_SERVICE_ROOT" ]]
)

if [[ -d /dev/shm && "$(stat -c '%d' "$ROOT")" != "$(stat -c '%d' /dev/shm)" ]]; then
  (
    configure_environment "$ROOT/cross-filesystem"
    cross_prefix="/dev/shm/termux-mcp-edge-test-$$"
    trap 'rm -rf -- "$cross_prefix"' EXIT
    export PREFIX="$cross_prefix"
    export TERMUX_MCP_SERVICE_ROOT="$PREFIX/var/service"
    export TERMUX_MCP_SERVICE_SHELL="$PREFIX/bin/sh"
    mkdir -p "$PREFIX"; make_shell "$PREFIX"
    artifact="$ROOT/cross-filesystem-server"; make_artifact "$artifact" 3.0.0; sha="$(artifact_sha "$artifact")"
    assert_fails bash "$SCRIPT" install --artifact "$artifact" --version 3.0.0 --sha256 "$sha"
    [[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/releases/3.0.0" && ! -e "$TERMUX_MCP_SERVICE_ROOT/mcp_runtime" ]]
  )
fi

(
  configure_environment "$ROOT/initial-start-failure"
  artifact="$ROOT/initial-start-failure-server"; make_artifact "$artifact" 3.0.0; sha="$(artifact_sha "$artifact")"
  if TERMUX_MCP_TEST_START_SEQUENCE=failure bash "$SCRIPT" install --artifact "$artifact" --version 3.0.0 --sha256 "$sha" >/dev/null 2>&1; then fail_test "initial start failure unexpectedly succeeded"; fi
  [[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/current" && ! -e "$TERMUX_MCP_DEPLOY_ROOT/releases/3.0.0" ]]
  [[ ! -e "$TERMUX_MCP_SERVICE_ROOT/mcp_runtime" ]]
  [[ -z "$(find "$TERMUX_MCP_DEPLOY_ROOT" -maxdepth 1 \( -name '.service-*' -o -name '.staging-*' \) -print -quit)" ]]
)

(
  configure_environment "$ROOT/initial-failure"
  artifact="$ROOT/initial-failure-server"; make_artifact "$artifact" 3.0.0; sha="$(artifact_sha "$artifact")"
  if TERMUX_MCP_TEST_PROBE_SEQUENCE=failure bash "$SCRIPT" install --artifact "$artifact" --version 3.0.0 --sha256 "$sha" >/dev/null 2>&1; then fail_test "unhealthy initial install unexpectedly succeeded"; fi
  [[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/current" && ! -e "$TERMUX_MCP_DEPLOY_ROOT/releases/3.0.0" ]]
  [[ ! -e "$TERMUX_MCP_SERVICE_ROOT/mcp_runtime" ]]
  [[ -z "$(find "$TERMUX_MCP_DEPLOY_ROOT" -maxdepth 1 \( -name '.service-*' -o -name '.staging-*' \) -print -quit)" ]]
)

service_snapshot="$(mktemp -d)"; cp -a "$SERVICE_DIR/." "$service_snapshot/"
TERMUX_MCP_TEST_STOP_SEQUENCE=failure assert_fails bash "$SCRIPT" uninstall
[[ -d "$TERMUX_MCP_DEPLOY_ROOT" && -d "$SERVICE_DIR" ]]
diff -ru "$service_snapshot" "$SERVICE_DIR"
rm -rf "$service_snapshot"

bash "$SCRIPT" uninstall
[[ ! -e "$TERMUX_MCP_DEPLOY_ROOT" && ! -e "$SERVICE_DIR" && -e "$TERMUX_MCP_CONFIG_ROOT/runtime.env" ]]
bash "$SCRIPT" install --artifact "$ARTIFACT_200" --version 2.0.0 --sha256 "$SHA_200"
bash "$SCRIPT" uninstall --purge-config
[[ ! -e "$TERMUX_MCP_CONFIG_ROOT" ]]

printf 'termux deployment tests passed\n'
