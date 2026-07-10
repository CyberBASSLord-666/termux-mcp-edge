# Operations Guide

## Purpose

This project runs a small Rust/Axum HTTP service on Android through Termux. The default compiled runtime exposes operational health/readiness endpoints and validates startup security posture. When built with the optional `mcp-runtime` feature, the service exposes a staged `/mcp` transport protected by bearer authentication and exact transport allowlist checks.

## Baseline Operating Model

- Rust single-binary service.
- Axum HTTP runtime.
- `GET /health` and `GET /ready` operational endpoints.
- Optional feature-gated `POST /mcp` staged transport.
- Static-token mode authenticates every `/mcp` request before transport validation or JSON-RPC handling.
- Explicit unauthenticated mode is loopback-development only and is rejected for non-loopback binds.
- Termux runtime.
- `termux-services` / runit supervision.
- Narrow dedicated filesystem safe-root default.
- MCP expansion remains staged by tool surface.

## Required Android Hardening

1. Set Termux battery usage to unrestricted.
2. Remove Termux from sleeping or deep-sleeping app lists.
3. Use `termux-wake-lock` only when persistent background operation is required.
4. On Android 14 or later, enable **Developer options → Disable child process restrictions**.
5. Avoid direct public port exposure. Prefer a named tunnel or VPN-bound endpoint only after authentication is configured and tested.

## Runtime Validation

Verify the unauthenticated operational probe:

```bash
curl -fsS http://127.0.0.1:8000/health
```

Expected response:

```text
ok
```

When the binary is built with `--features mcp-runtime`, validate both rejection and success paths:

1. A request without `Authorization` receives HTTP 401 and no discovery result.
2. A request with `Authorization: Bearer <configured-token>` reaches exact Host/Origin validation and the intended MCP path.
3. Tool discovery returns only the staged allowlisted tools.
4. Representative allowed and denied tool calls preserve safe-root, read-only, payload-limit, and dry-run boundaries.

For exact commands, follow [`docs/VALIDATION.md`](VALIDATION.md). Treat exact-head CI as a merge gate, and require Security when Cargo, lockfile, or Security workflow inputs change.

## Service Supervision

Install Termux services:

```bash
pkg install termux-services
```

Create a bearer-token file before enabling the service:

```bash
umask 077
openssl rand -hex 32 > "$HOME/.termux_mcp_token"
chmod 600 "$HOME/.termux_mcp_token"
```

The packaged runit script fails before starting the server if the token file is missing, empty, or whitespace-only. It reads the token without printing it and exports `MCP__AUTH__STATIC_TOKEN` for the server process.

Create or install the runit service script from `scripts/runit/mcp-server/run`, then start it:

```bash
sv-enable mcp-server
sv up mcp-server
sv status mcp-server
```

For a local authenticated smoke test, read the protected token into a temporary shell variable and clear it afterward:

```bash
MCP_TEST_TOKEN="$(cat "$HOME/.termux_mcp_token")"
curl -sS \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp
unset MCP_TEST_TOKEN
```

Do not use `set -x`, echo the token, paste it into issue text, or include it in screenshots.

## Filesystem Safe Roots

The default filesystem safe root is the dedicated Termux-home directory:

```text
/data/data/com.termux/files/home/mcp-files
```

This deliberately avoids broad Android shared-storage defaults such as `/storage/emulated/0` and `/sdcard`. Keep `MCP__FILE__SAFE_ROOTS` constrained to one or more dedicated project directories. Avoid all shared storage unless the deployment has a reviewed operational requirement and matching authorization controls.

Safe-root configuration is validated at startup. Empty safe-root lists, relative paths, and filesystem root `/` are rejected.

## Current MCP Tool Exposure

After authentication in static-token mode, current `tools/list` exposes:

1. `runtime_status` — deterministic staged runtime metadata and aggregate non-sensitive audit counters.
2. `platform_info` — non-sensitive platform metadata only.
3. `android_status` — read-only allowlisted Android/Termux status metadata with no Android API calls, shell fallback, or control behavior.
4. `project_service_status` — read-only allowlisted project-owned logical service metadata; the current service name is `mcp_runtime`.
5. `list_directory` — bounded safe-rooted directory listing.
6. `read_file` — bounded safe-rooted UTF-8 file reads.
7. `write_file` — safe-rooted, payload-bounded writes that default to dry-run unless `dry_run:false` is explicitly supplied.

Unauthorized clients must receive no tool list or tool result. The current runtime does not expose Android platform control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, or high-impact controls.

## Release Process

1. Run the exact CI command set: `cargo fmt`, workspace/all-target/all-feature Clippy, and workspace/all-target/all-feature tests.
2. Build both the default and `mcp-runtime` release postures.
3. Confirm the Security workflow passes when Cargo, lockfile, or Security workflow inputs change; otherwise document the path-filtered non-run.
4. Cross-compile with `scripts/cross_compile.sh` or validate the tag/manual-dispatch Android artifact.
5. Copy the release binary to `$HOME/bin/termux-mcp-server` on Android.
6. Confirm the protected token file exists with restrictive permissions.
7. Restart the runit service.
8. Verify `/health` returns `ok` and `/ready` reports the intended feature/auth posture.
9. If `mcp-runtime` is enabled, verify unauthenticated rejection, authenticated `tools/list`, and representative authenticated calls to `runtime_status`, `project_service_status`, and filesystem tools.
10. Do not claim readiness for Android control, command execution, or high-impact tools until their separate staged gates are implemented and validated.
