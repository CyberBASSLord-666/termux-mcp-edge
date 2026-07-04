# Operations Guide

## Purpose

This project runs a small Rust/Axum HTTP service on Android through Termux. The default compiled runtime exposes a health-check endpoint and validates startup security posture. When built with the optional `mcp-runtime` feature, the service exposes a staged `/mcp` transport with exact transport allow-list checks and a limited MCP tool surface.

## Baseline Operating Model

- Rust single-binary service.
- Axum HTTP runtime.
- `GET /health` endpoint for runtime liveness.
- Optional feature-gated `POST /mcp` staged transport.
- Termux runtime.
- `termux-services` / runit supervision.
- Bearer-token startup posture for constrained deployments.
- Narrow dedicated filesystem safe-root default.
- MCP restoration remains staged by tool surface.

## Required Android Hardening

1. Set Termux battery usage to unrestricted.
2. Remove Termux from sleeping or deep-sleeping app lists.
3. Use `termux-wake-lock` only when persistent background operation is required.
4. On Android 14 or later, enable **Developer options → Disable child process restrictions**.
5. Avoid direct public port exposure. Prefer a named tunnel or VPN-bound endpoint only after authentication is configured.

## Runtime Validation

```bash
curl -fsS http://127.0.0.1:8000/health
```

Expected response:

```text
ok
```

When the binary is built with `--features mcp-runtime`, validate exact transport checks, tool discovery, and representative tool calls before claiming MCP readiness for the enabled staged surface.

For repository-level validation, follow [`docs/VALIDATION.md`](VALIDATION.md). Treat CI and Security as merge gates before merging remediation branches.

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

The packaged runit script fails before starting the server if the token file is missing, empty, or whitespace-only. It does not supply a default bearer token.

Create or install the runit service script from `scripts/runit/mcp-server/run`, then start it:

```bash
sv-enable mcp-server
sv up mcp-server
sv status mcp-server
```

## Filesystem Safe Roots

The default filesystem safe root is the dedicated Termux-home directory:

```text
/data/data/com.termux/files/home/mcp-files
```

This deliberately avoids broad Android shared-storage defaults such as `/storage/emulated/0` and `/sdcard`. Keep `MCP__FILE__SAFE_ROOTS` constrained to one or more dedicated project directories. Avoid all shared storage unless the deployment has a reviewed operational requirement and matching authorization controls.

Safe-root configuration is validated at startup. Empty safe-root lists, relative paths, and filesystem root `/` are rejected.

## Current MCP Tool Exposure

When `mcp-runtime` is enabled, current `tools/list` exposes:

1. `runtime_status` — deterministic staged runtime metadata.
2. `platform_info` — non-sensitive platform metadata only.
3. `android_status` — read-only allowlisted Android/Termux status metadata with no Android API calls, shell fallback, or control behavior.
4. `project_service_status` — read-only allowlisted project-owned logical service metadata; the current service name is `mcp_runtime`.
5. `list_directory` — bounded safe-rooted directory listing.
6. `read_file` — bounded safe-rooted UTF-8 file reads.
7. `write_file` — safe-rooted, payload-bounded writes that default to dry-run unless `dry_run:false` is explicitly supplied.

The current runtime does not expose Android platform control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, or high-impact controls.

## Release Process

1. Validate with `cargo fmt`, `cargo clippy`, and `cargo test`.
2. Confirm the Security workflow passes when applicable, or document an accepted path-filtered non-run for docs-only changes.
3. Cross-compile with `scripts/cross_compile.sh`.
4. Copy the release binary to `$HOME/bin/termux-mcp-server` on Android.
5. Restart the runit service.
6. Verify `/health` returns `ok`.
7. If `mcp-runtime` is enabled, verify `/mcp` transport checks, `tools/list`, and representative calls to `runtime_status`, `project_service_status`, and filesystem tools.
8. Do not claim readiness for Android control, command execution, or high-impact tools until their separate staged gates are implemented and validated.
