# Operations Guide

## Purpose

This project currently runs a small Rust/Axum HTTP service on Android through Termux. The compiled runtime exposes a health-check endpoint and validates startup security posture. MCP transport and MCP tool endpoints are not exposed in the current runtime.

## Baseline Operating Model

- Rust single-binary service.
- Axum HTTP runtime.
- `GET /health` endpoint for runtime liveness.
- Termux runtime.
- `termux-services` / runit supervision.
- Bearer-token startup posture for constrained deployments.
- MCP transport restoration tracked separately from the current health-check runtime.

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

MCP-level validation is not applicable until MCP transport is restored. When MCP transport returns, add validation for tool discovery and at least one tool call before claiming MCP readiness.

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

## Current Tool Exposure

No MCP tools are exposed by the current compiled runtime. Filesystem and platform tool work must remain gated behind a future transport-restoration PR with explicit tests, documentation, and Security validation.

## Release Process

1. Validate with `cargo fmt`, `cargo clippy`, and `cargo test`.
2. Confirm the Security workflow passes.
3. Cross-compile with `scripts/cross_compile.sh`.
4. Copy the release binary to `$HOME/bin/termux-mcp-server` on Android.
5. Restart the runit service.
6. Verify `/health` returns `ok`.
7. Do not claim MCP readiness until MCP transport validation is added and passing.
