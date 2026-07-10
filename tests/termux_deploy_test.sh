#!/usr/bin/env bash
set -euo pipefail
IFS=$'\n\t'

ROOT="$(mktemp -d)"
trap 'rm -rf -- "$ROOT"' EXIT INT TERM
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCRIPT="$REPO_ROOT/scripts/termux_deploy.sh"
ARTIFACT="$ROOT/termux-mcp-server"
printf '#!/usr/bin/env sh\nexit 0\n' >"$ARTIFACT"
chmod 700 "$ARTIFACT"

export HOME="$ROOT/home"
export PREFIX="$ROOT/prefix"
export TERMUX_MCP_DEPLOY_ROOT="$ROOT/deploy"
export TERMUX_MCP_CONFIG_ROOT="$ROOT/config"
export TERMUX_MCP_SERVICE_ROOT="$ROOT/services"
export TERMUX_MCP_TEST_MODE=1
mkdir -p "$HOME" "$PREFIX"

assert_eq() {
  [[ "$1" == "$2" ]] || { printf 'assertion failed: expected %s, got %s\n' "$2" "$1" >&2; exit 1; }
}

bash -n "$SCRIPT"

bash "$SCRIPT" install --artifact "$ARTIFACT" --version 1.0.0
[[ -x "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0/termux-mcp-server" ]]
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"
[[ -x "$TERMUX_MCP_SERVICE_ROOT/mcp_runtime/run" ]]
head -n 1 "$TERMUX_MCP_SERVICE_ROOT/mcp_runtime/run" | grep -Fx "#!$PREFIX/bin/sh"
[[ -d "$TERMUX_MCP_CONFIG_ROOT" ]]
[[ "$(stat -c '%a' "$TERMUX_MCP_CONFIG_ROOT")" == "700" ]]

if bash "$SCRIPT" install --artifact "$ARTIFACT" --version 1.0.0 >/dev/null 2>&1; then
  printf 'duplicate install unexpectedly succeeded\n' >&2
  exit 1
fi

bash "$SCRIPT" upgrade --artifact "$ARTIFACT" --version 1.1.0
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"

if TERMUX_MCP_TEST_PROBE_RESULT=failure bash "$SCRIPT" upgrade --artifact "$ARTIFACT" --version 1.2.0 >/dev/null 2>&1; then
  printf 'unhealthy upgrade unexpectedly succeeded\n' >&2
  exit 1
fi
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"

bash "$SCRIPT" rollback
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"

# Re-establish a distinct rollback target after the failed-candidate recovery check.
rm -f "$TERMUX_MCP_DEPLOY_ROOT/previous"
ln -s "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0" "$TERMUX_MCP_DEPLOY_ROOT/previous"
bash "$SCRIPT" rollback
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/current")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"
assert_eq "$(readlink "$TERMUX_MCP_DEPLOY_ROOT/previous")" "$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"

status="$(bash "$SCRIPT" status)"
[[ "$status" == *"current=$TERMUX_MCP_DEPLOY_ROOT/releases/1.0.0"* ]]
[[ "$status" == *"previous=$TERMUX_MCP_DEPLOY_ROOT/releases/1.1.0"* ]]

if TERMUX_MCP_DEPLOY_ROOT=/ bash "$SCRIPT" status >/dev/null 2>&1; then
  printf 'unsafe root path unexpectedly accepted\n' >&2
  exit 1
fi

if bash "$SCRIPT" upgrade --artifact "$ARTIFACT" --version '../bad' >/dev/null 2>&1; then
  printf 'unsafe version unexpectedly accepted\n' >&2
  exit 1
fi

secret='never-print-this-token'
printf 'MCP__AUTH__STATIC_TOKEN=%s\n' "$secret" >"$TERMUX_MCP_CONFIG_ROOT/runtime.env"
output="$(bash "$SCRIPT" status)"
[[ "$output" != *"$secret"* ]]

bash "$SCRIPT" uninstall
[[ ! -e "$TERMUX_MCP_DEPLOY_ROOT" ]]
[[ ! -e "$TERMUX_MCP_SERVICE_ROOT/mcp_runtime" ]]
[[ -e "$TERMUX_MCP_CONFIG_ROOT/runtime.env" ]]

bash "$SCRIPT" install --artifact "$ARTIFACT" --version 2.0.0
bash "$SCRIPT" uninstall --purge-config
[[ ! -e "$TERMUX_MCP_CONFIG_ROOT" ]]

printf 'termux deployment tests passed\n'
