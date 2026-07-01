# Termux MCP Edge (Rust)

Termux MCP Edge is currently a hardened Rust/Axum HTTP service for Android Termux deployments. The default runtime exposes a health-check endpoint and enforces fail-closed authentication posture at startup.

The optional `mcp-runtime` feature now wires a minimal `/mcp` transport shell that validates `Host` and `Origin` headers before handling requests. It supports a staged MCP discovery contract with `initialize`, `tools/list`, and one deterministic read-only `server/info` tool. Filesystem tools, Android platform tools, command execution, and high-impact actions remain unavailable until later staged PRs validate each surface independently.

## Current Runtime Scope

- **Runtime:** Rust single binary using Axum.
- **Default HTTP endpoint:** `GET /health`.
- **Optional MCP transport shell:** `POST /mcp` when built with `--features mcp-runtime`.
- **Current MCP discovery:** `initialize` plus `tools/list` returning only the deterministic read-only `server/info` tool.
- **Current MCP tools:** `server/info` only; it returns static runtime metadata and does not read files, inspect Android state, execute commands, or mutate state.
- **Current filesystem/tool endpoints:** not exposed.
- **Authentication posture:** startup fails closed unless a non-empty static bearer token is configured or explicit localhost-only development mode is enabled.
- **Transport posture:** configured exact `Host` and browser `Origin` allow-lists are enforced before the staged MCP transport shell handles requests.
- **Filesystem safe-root default:** `/data/data/com.termux/files/home/mcp-files`, not broad shared storage.
- **Deployment target:** Termux on Android, supervised by `termux-services` / runit.

## Design Goals

- Memory efficiency and thermal resilience on mobile hardware.
- Fail-closed startup posture for networked deployments.
- Clear separation between transport liveness, tool discovery, read-only tool calls, and later stateful tool execution.
- Narrow default filesystem scope for any future file-capable tool restoration.
- Single-binary deployment optimized for `termux-services` and runit.
- CI and Security workflows as merge gates for every remediation branch.

## Security and Authentication

Set `MCP__AUTH__STATIC_TOKEN` to a strong random value before starting the service. Empty or whitespace-only values are rejected at startup.

Local unauthenticated development requires both conditions:

```bash
export MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true
export MCP__SERVER__HOST=localhost
```

This opt-in is rejected for non-loopback bind addresses and must not be used with tunnels, LAN exposure, reverse proxies, or shared network access.

Browser-reachable MCP transport requests are additionally constrained by exact transport allow-lists:

```bash
export MCP__TRANSPORT__ALLOWED_HOSTS='["localhost:8000"]'
export MCP__TRANSPORT__ALLOWED_ORIGINS='["http://localhost:8000"]'
```

`MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=true` is only for explicitly reviewed non-browser clients that cannot send an `Origin` header.

## Filesystem Safe Roots

The built-in filesystem safe-root default is intentionally narrow:

```text
/data/data/com.termux/files/home/mcp-files
```

The service no longer defaults to broad Android shared-storage roots such as `/storage/emulated/0` or `/sdcard`. If future filesystem tools are restored, operators should keep `MCP__FILE__SAFE_ROOTS` limited to a dedicated project directory and avoid granting all shared storage unless there is a specific reviewed need.

## Architecture

- **Language:** Rust edition 2021.
- **HTTP framework:** Axum.
- **Default compiled transport:** health-check HTTP route only.
- **Optional MCP transport shell:** feature-gated `/mcp` route with transport security validation, `initialize`, `tools/list`, and the read-only `server/info` tool.
- **MCP framework dependency:** optional `rmcp` dependency behind `mcp-runtime`.
- **Supervision:** `termux-services` / runit.
- **Networking:** bind to localhost by default; prefer VPN or named tunnel only after authentication is configured.

## Runtime Roadmap

MCP runtime restoration is staged in [`docs/MCP_RUNTIME_ROADMAP.md`](docs/MCP_RUNTIME_ROADMAP.md). The roadmap keeps transport restoration, tool discovery, read-only tools, filesystem tools, Android platform tools, and high-impact tools in separate validation tracks.

## Quick Build

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo build --release
```

Build the staged transport shell explicitly:

```bash
cargo build --features mcp-runtime
```

For Android cross-compilation:

```bash
rustup target add aarch64-linux-android
ANDROID_NDK_HOME=/path/to/android-ndk ./scripts/cross_compile.sh
```

Transfer the resulting binary from `target/aarch64-linux-android/release/termux-mcp-server` to the device.

## Termux Setup

Install the supervisor:

```bash
pkg install termux-services
```

Create a local bearer-token file before enabling the runit service:

```bash
umask 077
openssl rand -hex 32 > "$HOME/.termux_mcp_token"
chmod 600 "$HOME/.termux_mcp_token"
```

The packaged runit script fails before starting the service if the token file is missing, empty, or whitespace-only.

Start the service:

```bash
sv-enable mcp-server
sv up mcp-server
sv status mcp-server
```

## Runtime Validation

```bash
curl -fsS http://127.0.0.1:8000/health
```

Expected response:

```text
ok
```

With `mcp-runtime` enabled, the staged transport shell should be reachable only after exact transport checks pass:

```bash
curl -i \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  http://127.0.0.1:8000/mcp
```

An empty body returns `501 Not Implemented` to show that the shell is reachable but not a full stateful tool runtime.

The staged tool-discovery contract exposes only the deterministic read-only `server/info` tool:

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp
```

Expected response includes only `server/info`:

```json
{"id":1,"jsonrpc":"2.0","result":{"tools":[{"description":"Returns deterministic read-only metadata about this Termux MCP Edge runtime.","inputSchema":{"additionalProperties":false,"properties":{},"type":"object"},"name":"server/info"}]}}
```

Call the read-only metadata tool:

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"server/info","arguments":{}}}' \
  http://127.0.0.1:8000/mcp
```

Expected `structuredContent` includes `readOnly: true`, `filesystem: false`, `androidPlatform: false`, `commandExecution: false`, and `highImpactActions: false`.

## MCP Transport Restoration Gate

Do not claim MCP readiness until all of the following are true:

1. A compatible MCP transport implementation is restored intentionally.
2. Dependency advisories for the chosen MCP stack are closed or documented with an accepted exception.
3. CI and Security workflows are green on the exact PR head.
4. A smoke test proves MCP tool discovery and at least one tool call.
5. README, operations, security, and validation docs match the runtime behavior.

See `docs/VALIDATION.md` for repository validation expectations.
