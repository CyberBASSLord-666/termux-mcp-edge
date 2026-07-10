#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/termux_deploy.sh"

assert_eq() {
  [[ "$1" == "$2" ]] || {
    printf 'assertion failed: expected %s, got %s\n' "$2" "$1" >&2
    exit 1
  }
}

assert_fails() {
  if "$@" >/dev/null 2>&1; then
    printf 'command unexpectedly succeeded:' >&2
    printf ' %q' "$@" >&2
    printf '\n' >&2
    exit 1
  fi
}

make_shell() {
  local prefix="$1"
  mkdir -p "$prefix/bin"
  cat >"$prefix/bin/sh" <<'EOF'
#!/bin/sh
exec /bin/sh "$@"
EOF
  chmod 700 "$prefix/bin/sh"
}

make_config() {
  local config_root="$1"
  mkdir -p "$config_root"
  chmod 700 "$config_root"
  cat >"$config_root/runtime.env" <<'EOF'
MCP__AUTH__STATIC_TOKEN=test-static-token
MCP__SERVER__HOST=127.0.0.1
MCP__SERVER__PORT=8000
MCP__TRANSPORT__ALLOWED_HOSTS=localhost:8000,127.0.0.1:8000
MCP__TRANSPORT__ALLOWED_ORIGINS=http://localhost:8000,http://127.0.0.1:8000
RUST_LOG=termux_mcp_server=info
EOF
  chmod 600 "$config_root/runtime.env"
}

make_artifact() {
  local path="$1" version="$2"
  cat >"$path" <<EOF
#!/bin/sh
if [ "\${1:-}" = "--version" ]; then
  printf 'termux-mcp-server %s\\n' '$version'
  exit 0
fi
exit 0
EOF
  chmod 700 "$path"
}

artifact_sha() {
  sha256sum -- "$1" | awk '{print $1}'
}

configure_environment() {
  local root="$1"
  export HOME="$root/home"
  export PREFIX="$root/prefix"
  export TERMUX_MCP_DEPLOY_ROOT="$HOME/.local/share/termux-mcp-edge"
  export TERMUX_MCP_CONFIG_ROOT="$HOME/.config/termux-mcp-edge"
  export TERMUX_MCP_SERVICE_ROOT="$PREFIX/var/service"
  export TERMUX_MCP_SERVICE_SHELL="$PREFIX/bin/sh"
  export TERMUX_MCP_TEST_MODE=1
  export TERMUX_MCP_TEST_PROBE_SEQUENCE=success
  unset TERMUX_MCP_ALLOW_UNVERIFIED_ARTIFACT TERMUX_MCP_DRY_RUN
  mkdir -p "$HOME" "$PREFIX"
  make_shell "$PREFIX"
  make_config "$TERMUX_MCP_CONFIG_ROOT"
}

bash -n "$SCRIPT"
configure_environment "$ROOT/main"

ARTIFACT_100="$ROOT/termux-mcp-server-1.0.0"
ARTIFACT_110="$ROOT/termux-mcp-server-1.1.0"
ARTIFACT_120="$ROOT/termux-mcp-server-1.2.0"
ARTIFACT_130="$ROOT/termux-mcp-server-1.3.0"
ARTIFACT_200="$ROOT/termux-mcp-server-2.0.0"
make_artifact "$ARTIFACT_100" 1.0.0
make_artifact "$ARTIFACT_110" 1.1.0
make_artifact "$ARTIFACT_120" 1.2.0
make_artifact "$ARTIFACT_130" 1.3.0
make_artifact "$ARTIFACT_200" 2.0.0
SHA_100="$(artifact_sha "$ARTIFACT_100")"
SHA_110="$(artifact_sha "$ARTIFACT_110")"
SHA_120="$(artifact_sha "$ARTIFACT_120")"
SHA_130="$(artifact_sha "$ARTIFACT_130")"
SHA_200="$(artifact_sha "$ARTIFACT_200")"
BAD_SHA="0000000000000000000000000000000000000000000000000000000000000000"

bash "$SCRIPT" install --artifact "$ARTIFACT_100" --version 1.0.0 --sha256 "$SHA_100"
[[ -x "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0/termux-mcp-server" ]]
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"
[[ -x "$TERMUX_MCP_SERVICE_ROOT/mcp_runtime/run" ]]
[[ ! -e "$TERMUX_MCP_SERVICE_ROOT/mcp_runtime/log" ]]
head -n 1 "$TERMUX_MCP_SERVICE_ROOT/mcp_runtime/run" | grep -Fx "#!$PREFIX/bin/sh"
[[ "$(stat -c '%a' "$TERMUX_MCP_CONFIG_ROOT")" == "700" ]]
[[ "$(stat -c '%a' "$TERMUX_MCP_CONFIG_ROOT/runtime.env")" == "600" ]]

PWNED="$ROOT/config-was-executed"
printf 'RUST_BACKTRACE=$(touch %s)\n' "$PWNED" >>"$TERMUX_MCP_CONFIG_ROOT/runtime.env"
"$TERMUX_MCP_SERVICE_ROOT/mcp_runtime/run"
[[ ! -e "$PWNED" ]]

assert_fails bash "$SCRIPT" install --artifact "$ARTIFACT_100" --version 1.0.0 --sha256 "$SHA_100"
assert_fails bash "$SCRIPT" install --artifact "$ARTIFACT_110" --version 1.1.0 --sha256 "$SHA_110"
assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_110" --version 1.1.0
assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_110" --version 1.1.0 --sha256 "$BAD_SHA"
assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_110" --version 9.9.9 --sha256 "$SHA_110"

bash "$SCRIPT" upgrade --artifact "$ARTIFACT_110" --version 1.1.0 --sha256 "$SHA_110"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"

if TERMUX_MCP_TEST_PROBE_SEQUENCE=failure,success bash "$SCRIPT" upgrade --artifact "$ARTIFACT_120" --version 1.2.0 --sha256 "$SHA_120" >/dev/null 2>&1; then
  printf 'unhealthy upgrade unexpectedly succeeded\n' >&2
  exit 1
fi
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"
[[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/releases/1.2.0" ]]

if TERMUX_MCP_TEST_PROBE_SEQUENCE=failure,success bash "$SCRIPT" rollback >/dev/null 2>&1; then
  printf 'unhealthy rollback unexpectedly succeeded\n' >&2
  exit 1
fi
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"

bash "$SCRIPT" rollback
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"

LOCK_DIR="${TERMUX_MCP_DEPLOY_ROOT}.deploy-lock"
mkdir -p "$LOCK_DIR"
printf '%s\n' "$$" >"$LOCK_DIR/owner.pid"
assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_120" --version 1.2.0 --sha256 "$SHA_120"
rm -rf "$LOCK_DIR"
mkdir -p "$LOCK_DIR"
printf '%s\n' 999999 >"$LOCK_DIR/owner.pid"
bash "$SCRIPT" upgrade --artifact "$ARTIFACT_120" --version 1.2.0 --sha256 "$SHA_120"
[[ ! -e "$LOCK_DIR" ]]
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.2.0"

current_before="$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")"
bash "$SCRIPT" upgrade --artifact "$ARTIFACT_130" --version 1.3.0 --sha256 "$SHA_130" --dry-run >/dev/null
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$current_before"
[[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/releases/1.3.0" ]]

status="$(bash "$SCRIPT" status)"
[[ "$status" == *"current=$TERMUX_MCP_DEPLOY_ROOT/releases/1.2.0"* ]]
[[ "$status" != *"test-static-token"* ]]

previous_target="$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")"
rm -f "$TERMUX_MCP_DEPLOY_ROOT/previous"
ln -s /tmp "$TERMUX_MCP_DEPLOY_ROOT/previous"
assert_fails bash "$SCRIPT" rollback
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.2.0"
rm -f "$TERMUX_MCP_DEPLOY_ROOT/previous"
ln -s "$previous_target" "$TERMUX_MCP_DEPLOY_ROOT/previous"

chmod 644 "$TERMUX_MCP_CONFIG_ROOT/runtime.env"
assert_fails bash "$SCRIPT" rollback
chmod 600 "$TERMUX_MCP_CONFIG_ROOT/runtime.env"
printf 'PATH=/tmp\n' >>"$TERMUX_MCP_CONFIG_ROOT/runtime.env"
assert_fails bash "$SCRIPT" rollback
sed -i '/^PATH=/d' "$TERMUX_MCP_CONFIG_ROOT/runtime.env"

assert_fails env TERMUX_MCP_DEPLOY_ROOT="$HOME" bash "$SCRIPT" status
assert_fails env TERMUX_MCP_CONFIG_ROOT="$HOME/bad path" bash "$SCRIPT" status
assert_fails env TERMUX_MCP_SERVICE_ROOT="$ROOT/outside-prefix" bash "$SCRIPT" status
assert_fails bash "$SCRIPT" upgrade --artifact "$ARTIFACT_130" --version '../bad' --sha256 "$SHA_130"

(
  configure_environment "$ROOT/initial-failure"
  local_artifact="$ROOT/initial-failure-artifact"
  make_artifact "$local_artifact" 3.0.0
  local_sha="$(artifact_sha "$local_artifact")"
  if TERMUX_MCP_TEST_PROBE_SEQUENCE=failure bash "$SCRIPT" install --artifact "$local_artifact" --version 3.0.0 --sha256 "$local_sha" >/dev/null 2>&1; then
    printf 'unhealthy initial install unexpectedly succeeded\n' >&2
    exit 1
  fi
  [[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/current" ]]
  [[ ! -e "$TERMUX_MCP_DEPLOY_ROOT/releases/3.0.0" ]]
)

bash "$SCRIPT" uninstall
[[ ! -e "$TERMUX_MCP_DEPLOY_ROOT" ]]
[[ ! -e "$TERMUX_MCP_SERVICE_ROOT/mcp_runtime" ]]
[[ -e "$TERMUX_MCP_CONFIG_ROOT/runtime.env" ]]
bash "$SCRIPT" install --artifact "$ARTIFACT_200" --version 2.0.0 --sha256 "$SHA_200"
bash "$SCRIPT" uninstall --purge-config
[[ ! -e "$TERMUX_MCP_CONFIG_ROOT" ]]

printf 'termux deployment tests passed\n'
