# Termux Deployment, Upgrade, and Recovery

This procedure installs the cross-compiled `termux-mcp-server` binary into a project-owned, versioned layout and manages only the `mcp_runtime` runit service.

## Prerequisites

Install Termux packages used by the runtime and deployment manager:

```bash
pkg update
pkg install curl file termux-services
```

Build or download the `aarch64-linux-android` artifact, then make the deployment manager executable:

```bash
chmod 700 scripts/termux_deploy.sh
```

The default layout is:

```text
~/.local/share/termux-mcp-edge/
  releases/<version>/termux-mcp-server
  current -> releases/<active-version>
  previous -> releases/<rollback-version>
~/.config/termux-mcp-edge/runtime.env
$PREFIX/var/service/mcp_runtime/run
```

Configuration and bearer-token material remain outside versioned releases. The deployment manager creates configuration directories with mode `0700` and never prints `runtime.env`.

## Initial install

Create `~/.config/termux-mcp-edge/runtime.env` with mode `0600` before starting a network-accessible runtime:

```bash
install -d -m 700 ~/.config/termux-mcp-edge
cat > ~/.config/termux-mcp-edge/runtime.env <<'EOF'
MCP__AUTH__STATIC_TOKEN=replace-with-a-strong-random-token
MCP__SERVER__HOST=127.0.0.1
MCP__SERVER__PORT=8000
MCP__TRANSPORT__ALLOWED_HOSTS=["localhost:8000"]
MCP__TRANSPORT__ALLOWED_ORIGINS=["http://localhost:8000"]
EOF
chmod 600 ~/.config/termux-mcp-edge/runtime.env
```

Install a validated artifact:

```bash
scripts/termux_deploy.sh install \
  --artifact target/aarch64-linux-android/release/termux-mcp-server \
  --version 0.5.1
```

The manager validates the artifact, stages it in a version-specific directory, writes the project-owned runit service, atomically switches `current`, starts the service, and checks `/health` and `/ready`.

## Upgrade

```bash
scripts/termux_deploy.sh upgrade --artifact /path/to/new/termux-mcp-server --version 0.6.0
```

The previous active release is retained through the `previous` symlink. If the candidate fails health or readiness checks, the manager restores and restarts the previous release automatically.

## Explicit rollback

```bash
scripts/termux_deploy.sh rollback
```

Rollback swaps `current` and `previous`, restarts the project-owned service, and requires the restored runtime to pass health and readiness probes.

## Status and recovery

```bash
scripts/termux_deploy.sh status
sv status "$PREFIX/var/service/mcp_runtime"
curl -fsS http://127.0.0.1:8000/health
curl -fsS http://127.0.0.1:8000/ready
```

For interrupted deployments, re-run the same install or upgrade with a new version identifier after removing only an abandoned `.staging-*` directory under the project deployment root. Never remove the persistent configuration directory during routine recovery.

## Uninstall

Remove releases and the project-owned runit service while preserving configuration:

```bash
scripts/termux_deploy.sh uninstall
```

Explicitly remove persistent configuration and token material:

```bash
scripts/termux_deploy.sh uninstall --purge-config
```

## CI and dry-run validation

CI uses an isolated test root and does not require Android or a live runit daemon:

```bash
bash tests/termux_deploy_test.sh
```

Inspect planned mutations without applying them:

```bash
TERMUX_MCP_DRY_RUN=1 scripts/termux_deploy.sh install --artifact /path/to/binary --version test
```

## On-device release checklist

1. Confirm the artifact came from the expected exact commit or release tag.
2. Confirm `file termux-mcp-server` reports an AArch64 Android-compatible executable.
3. Confirm `runtime.env` is mode `0600` and contains a non-empty static bearer token.
4. Install or upgrade through `termux_deploy.sh`.
5. Confirm `status`, `sv status`, `/health`, and `/ready` succeed.
6. Run authenticated MCP discovery using the operator-validation procedure.
7. Deliberately test rollback before treating a release as production-ready.
8. Preserve the previous release until the new release has completed sustained device validation.
