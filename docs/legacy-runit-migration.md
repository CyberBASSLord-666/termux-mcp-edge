# Legacy runit service migration

The supported Termux service is `mcp_runtime`, managed only by `scripts/termux_deploy.sh`. The historical `mcp-server` runner is retired because it used a different binary path and token-file contract and could compete for the same listener.

## Detect the old service

```bash
test -e "$PREFIX/var/service/mcp-server" && echo "legacy service present"
sv status "$PREFIX/var/service/mcp-server" 2>/dev/null || true
```

Do not start `mcp_runtime` while the legacy service is still running.

## Retire it safely

Review the operation first:

```bash
TERMUX_MCP_DRY_RUN=1 bash scripts/retire_legacy_runit.sh
```

Then retire the old service:

```bash
bash scripts/retire_legacy_runit.sh
```

The helper fails closed if the legacy service path is a symlink, if `sv` is unavailable, or if the process cannot be confirmed down. It removes only the legacy service directory. It does not delete releases, the canonical service, canonical configuration, or `$HOME/.termux_mcp_token`.

## Migrate configuration

The canonical deployment reads:

```text
$HOME/.config/termux-mcp-edge/runtime.env
```

If `$HOME/.termux_mcp_token` exists, copy its token value into `MCP__AUTH__STATIC_TOKEN` in the canonical `runtime.env` using restrictive permissions. Do not source or echo the token in logs.

After verifying the canonical service starts successfully, remove the obsolete token file manually:

```bash
chmod 600 "$HOME/.config/termux-mcp-edge/runtime.env"
rm -f -- "$HOME/.termux_mcp_token"
```

## Install or validate the canonical service

Use the versioned deployment manager documented in `docs/termux-deployment.md`. Confirm only one project service exists and owns the configured listener:

```bash
sv status "$PREFIX/var/service/mcp_runtime"
test ! -e "$PREFIX/var/service/mcp-server"
curl -fsS http://127.0.0.1:8000/health
curl -fsS http://127.0.0.1:8000/ready
```

Never recreate `scripts/runit/mcp-server/run` or run both service names concurrently.
