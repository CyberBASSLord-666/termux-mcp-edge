# Operations Guide

## Purpose

This project is designed to run a small, secure MCP server on Android through Termux, preferably on high-end devices such as the Samsung Galaxy Z Fold 6.

## Baseline operating model

- Rust single-binary service.
- Termux runtime.
- `termux-services` / runit supervision.
- Optional Cloudflare named tunnel for remote ingress.
- Bearer-token authentication for constrained deployments, with OAuth 2.1 / PKCE recommended for enterprise exposure.

## Required Android hardening

1. Set Termux battery usage to unrestricted.
2. Remove Termux from sleeping or deep-sleeping app lists.
3. Use `termux-wake-lock` only when persistent background operation is required.
4. On Android 14 or later, enable **Developer options → Disable child process restrictions**.
5. Avoid direct public port exposure. Prefer a named Cloudflare Tunnel or a VPN-bound endpoint.

## Runtime validation

```bash
curl -fsS http://127.0.0.1:8000/health
```

For MCP-level validation, use the MCP Inspector from a trusted desktop environment and authenticate with the configured bearer token or OAuth flow.

## Service supervision

Install Termux services:

```bash
pkg install termux-services
```

Create or install the runit service script from `scripts/runit/mcp-server/run`, then start it:

```bash
sv-enable mcp-server
sv up mcp-server
sv status mcp-server
```

## Release process

1. Validate with `cargo fmt`, `cargo clippy`, and `cargo test`.
2. Cross-compile with `scripts/cross_compile.sh`.
3. Copy the release binary to `$HOME/bin/termux-mcp-server` on Android.
4. Restart the runit service.
5. Verify `/health` and MCP tool listing.
