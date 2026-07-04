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
- MCP transport restoration tracked in staged PRs, with Android/platform tools, command execution, and high-impact controls still disabled.

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

When built with `--features mcp-runtime`, MCP-level validation must include transport header checks, `tools/list`, `runtime_status`, `platform_info`, and at least one safe-rooted filesystem tool call before claiming MCP readiness. The staged MCP runtime currently exposes `runtime_status`, read-only non-sensitive `platform_info`, safe-rooted `list_directory`, bounded safe-rooted `read_file`, and safe-rooted `write_file` with dry-run-by-default behavior. Mutating writes require explicit `"dry_run": false` and remain constrained to configured safe roots.

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

Safe-root configuration is validated at startup. Empty safe-root lists, relative paths, and filesystem root `/` are rejected. Staged filesystem tools must reject traversal and outside-root paths.

## Current Tool Exposure

The default compiled runtime exposes no MCP tools unless built with the optional `mcp-runtime` feature. With `mcp-runtime` enabled, the current staged MCP surface exposes:

- `runtime_status`: deterministic runtime metadata.
- `platform_info`: non-sensitive read-only OS, architecture, platform family, available parallelism, and package-version metadata.
- `list_directory`: bounded safe-rooted directory metadata without file-content reads.
- `read_file`: bounded safe-rooted UTF-8 file reads.
- `write_file`: safe-rooted UTF-8 writes that default to dry-run; mutation requires explicit `"dry_run": false`.

Android platform APIs, process inspection, shell or command execution, and high-impact controls remain disabled and must be restored only through later staged PRs with explicit tests, documentation, and gate review.

## Release Process

1. Validate with `cargo fmt`, `cargo clippy`, and `cargo test`.
2. Confirm the Security workflow passes or is intentionally path-filtered out for a change that does not touch dependencies, workflows, lockfiles, or runtime-sensitive surfaces.
3. Cross-compile with `scripts/cross_compile.sh`.
4. Copy the release binary to `$HOME/bin/termux-mcp-server` on Android.
5. Restart the runit service.
6. Verify `/health` returns `ok`.
7. When using `--features mcp-runtime`, verify transport security headers, tool discovery, `runtime_status`, `platform_info`, and representative safe-rooted filesystem calls before claiming MCP readiness.
