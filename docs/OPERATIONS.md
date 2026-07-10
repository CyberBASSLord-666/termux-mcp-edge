# Operations Guide

## Purpose

This project runs a small Rust/Axum HTTP service on Android through Termux. The default compiled runtime exposes operational health/readiness endpoints and validates startup security posture. When built with the optional `mcp-runtime` feature, the service exposes a staged `/mcp` transport protected by bearer authentication, mobile-conscious request resource limits, and exact transport allowlist checks.

## Baseline Operating Model

- Rust single-binary service.
- Axum HTTP runtime.
- `GET /health` and `GET /ready` operational endpoints.
- Optional feature-gated `POST /mcp` staged transport.
- Static-token mode authenticates every `/mcp` request before resource-limit accounting, transport validation, or JSON-RPC handling.
- Explicit unauthenticated mode is loopback-development only and is rejected for non-loopback binds.
- Four concurrent authenticated MCP requests by default.
- Thirty-second total MCP request timeout by default.
- Two-MiB MCP request-body ceiling by default.
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
6. Keep request-limit defaults unless measured workload demonstrates a need for a reviewed increase.

## Runtime Validation

Verify the unauthenticated operational probe:

```bash
curl -fsS http://127.0.0.1:8000/health
```

Expected response:

```text
ok
```

Inspect readiness metadata:

```bash
curl -fsS http://127.0.0.1:8000/ready | jq
```

When `mcp-runtime` is enabled, `mcp_request_limits` should report the active concurrency, timeout, and body-size values. Readiness metadata is non-sensitive and must not expose tokens, raw paths, or tool output.

When the binary is built with `--features mcp-runtime`, validate rejection and success paths:

1. A request without `Authorization` receives HTTP 401 and no discovery result.
2. A request with `Authorization: Bearer <configured-token>` reaches request-limit and exact Host/Origin validation.
3. Tool discovery returns only the staged allowlisted tools.
4. Representative allowed and denied tool calls preserve safe-root, read-only, payload-limit, and dry-run boundaries.
5. A body over the configured ceiling receives HTTP 413.
6. Saturated concurrency receives HTTP 503 with `Retry-After: 1`.
7. A request exceeding the configured duration receives HTTP 504.

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

Optional request-limit overrides may be exported by the runit service only after measured validation:

```bash
export MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS=4
export MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS=30
export MCP__TRANSPORT__MAX_BODY_BYTES=2097152
```

Validated ranges are:

- concurrency: `1–64`;
- timeout: `1–300` seconds;
- body size: `1024–8388608` bytes.

Values outside those ranges prevent startup. Increasing concurrency and body size together increases possible memory pressure, so evaluate the product of both settings on the target Android device rather than tuning either in isolation.

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

## Request-Limit Failure Semantics

Resource-limit failures are transport responses, not MCP tool results:

- HTTP 413 / `mcp_request_body_too_large`: request body exceeds the configured ceiling.
- HTTP 503 / `mcp_concurrency_limit_reached`: all concurrency permits are occupied; retry only after the advertised delay.
- HTTP 504 / `mcp_request_timeout`: body extraction or MCP dispatch exceeded the configured duration.

All three responses include `Cache-Control: no-store` and omit request content, tokens, paths, and tool arguments.

Authentication is the outermost gate in static-token mode. Unauthenticated traffic receives HTTP 401 before consuming MCP concurrency permits or body-buffer capacity.

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
8. Verify `/health` returns `ok` and `/ready` reports the intended feature, auth posture, and request limits.
9. If `mcp-runtime` is enabled, verify unauthenticated rejection, authenticated `tools/list`, request-limit failure responses, and representative authenticated calls to `runtime_status`, `project_service_status`, and filesystem tools.
10. Do not claim readiness for Android control, command execution, or high-impact tools until their separate staged gates are implemented and validated.
