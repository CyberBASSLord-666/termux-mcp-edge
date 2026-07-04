# Operations Guide

## Purpose

This project currently runs a small Rust/Axum HTTP service on Android through Termux. The default compiled runtime exposes a health-check endpoint and validates startup security posture. The optional `mcp-runtime` feature exposes the staged `/mcp` transport shell after exact transport-security checks pass.

## Baseline Operating Model

- Rust single-binary service.
- Axum HTTP runtime.
- `GET /health` endpoint for runtime liveness.
- Optional feature-gated `/mcp` transport shell.
- Termux runtime.
- `termux-services` / runit supervision.
- Bearer-token startup posture for constrained deployments.
- Narrow dedicated filesystem safe-root default.
- MCP transport restoration tracked in staged PRs, with Android platform API/control tools, command execution, and high-impact controls still disabled.

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

When built with `--features mcp-runtime`, MCP-level validation must verify transport security headers, tool discovery, and at least one low-risk tool call before claiming MCP readiness.

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp
```

The current staged discovery surface is limited to `runtime_status`, `platform_info`, `android_status`, `list_directory`, `read_file`, and `write_file`. `android_status` is read-only status metadata only. Android platform APIs/control tools, command execution, process inspection, shell fallback, and high-impact controls remain unavailable.

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

This deliberately avoids broad Android shared-storage defaults such as `/storage/emulated/0` and `/sdcard`. The staged MCP filesystem surface exposes bounded directory listing, bounded UTF-8 file reads, and safe-rooted write requests that default to dry-run. Mutating writes require explicit `"dry_run": false`. Keep `MCP__FILE__SAFE_ROOTS` constrained to one or more dedicated project directories. Avoid all shared storage unless the deployment has a reviewed operational requirement and matching authorization controls.

Safe-root configuration is validated at startup. Empty safe-root lists, relative paths, and filesystem root `/` are rejected.

## Current Tool Exposure

The default compiled runtime exposes no MCP tools. When built with `--features mcp-runtime`, the staged tool surface is limited to:

- `runtime_status`: deterministic read-only runtime metadata.
- `platform_info`: non-sensitive read-only platform metadata.
- `android_status`: read-only allowlisted Android/Termux status metadata that confirms Android API access, Android control, shell fallback, command execution, and high-impact controls are disabled.
- `list_directory`: safe-rooted bounded directory listing.
- `read_file`: safe-rooted bounded UTF-8 file reads.
- `write_file`: safe-rooted writes that default to dry-run and require explicit `"dry_run": false` to mutate.

Further filesystem, Android/platform, command-capable, and high-impact tool work must remain gated behind staged PRs with explicit tests, documentation, and Security validation.

## Release Process

1. Validate with `cargo fmt`, `cargo clippy`, and `cargo test`.
2. Confirm the Security workflow passes.
3. Cross-compile with `scripts/cross_compile.sh`.
4. Copy the release binary to `$HOME/bin/termux-mcp-server` on Android.
5. Restart the runit service.
6. Verify `/health` returns `ok`.
7. Validate `/mcp` only for builds that explicitly enable `--features mcp-runtime`.
8. Do not claim broad MCP readiness until each restored runtime surface has staged validation.
