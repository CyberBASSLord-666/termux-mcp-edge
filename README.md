# Termux MCP Edge (Rust)

Termux MCP Edge is currently a hardened Rust/Axum HTTP service for Android Termux deployments. The default runtime exposes a health-check endpoint and enforces fail-closed authentication posture at startup.

The optional `mcp-runtime` feature wires a staged `/mcp` transport that validates `Host` and browser `Origin` headers before handling requests. It currently supports `initialize`, `tools/list`, deterministic read-only `runtime_status`, non-sensitive read-only `platform_info`, read-only allowlisted `android_status`, safe-rooted `list_directory`, bounded safe-rooted UTF-8 `read_file`, default-dry-run safe-rooted `write_file`, and read-only allowlisted `project_service_status`. Android platform control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, and high-impact actions remain unavailable until later staged PRs validate each surface independently.

## Current Runtime Scope

- **Runtime:** Rust single binary using Axum.
- **Default HTTP endpoint:** `GET /health`.
- **Optional MCP transport shell:** `POST /mcp` when built with `--features mcp-runtime`.
- **Current MCP discovery:** `initialize` plus `tools/list` returning `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file`.
- **Current MCP tools:** deterministic read-only runtime metadata, non-sensitive platform metadata, read-only allowlisted Android/Termux status metadata, read-only allowlisted project-owned service status metadata, bounded safe-rooted directory listing, bounded safe-rooted UTF-8 file reads, and default-dry-run safe-rooted file writes.
- **Current filesystem/tool endpoints:** directory listing and file reads are bounded to configured safe roots; `write_file` is exposed with explicit safe-root, payload-size, and dry-run-by-default controls.
- **Authentication posture:** startup fails closed unless a non-empty static bearer token is configured or explicit localhost-only development mode is enabled.
- **Transport posture:** configured exact `Host` and browser `Origin` allow-lists are enforced before the staged MCP transport handles requests.
- **Filesystem safe-root default:** `/data/data/com.termux/files/home/mcp-files`, not broad shared storage.
- **Project service status scope:** `project_service_status` reports only project-owned logical services from the explicit allowlist; the current public service name is `mcp_runtime`.
- **Deployment target:** Termux on Android, supervised by `termux-services` / runit.

## Design Goals

- Memory efficiency and thermal resilience on mobile hardware.
- Fail-closed startup posture for networked deployments.
- Clear separation between transport liveness, tool discovery, low-risk read-only tools, filesystem listing, file reads, file writes, project-owned service status, and later higher-impact tool execution.
- Narrow default filesystem scope for file-capable tools.
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

The service does not default to broad Android shared-storage roots such as `/storage/emulated/0` or `/sdcard`. The staged filesystem MCP surface exposes bounded directory listing, bounded UTF-8 file reads, and default-dry-run writes beneath configured safe roots. Operators should keep `MCP__FILE__SAFE_ROOTS` limited to a dedicated project directory and avoid granting all shared storage unless there is a specific reviewed need.

## Architecture

- **Language:** Rust edition 2021.
- **HTTP framework:** Axum.
- **Default compiled transport:** health-check HTTP route only.
- **Optional MCP transport shell:** feature-gated `/mcp` route with transport security validation, `initialize`, `tools/list`, `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file`.
- **MCP framework dependency:** none; the staged runtime uses a minimal internal JSON-RPC transport shell.
- **Supervision:** `termux-services` / runit.
- **Networking:** bind to localhost by default; prefer VPN or named tunnel only after authentication is configured.

## Runtime Roadmap

MCP runtime restoration is staged in [`docs/MCP_RUNTIME_ROADMAP.md`](docs/MCP_RUNTIME_ROADMAP.md). The roadmap keeps transport restoration, tool discovery, read-only tools, filesystem tools, Android/platform status and control, project-owned service status, command execution, and high-impact tools in separate validation tracks.

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

With `mcp-runtime` enabled, the staged transport should be reachable only after exact transport checks pass:

```bash
curl -i \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  http://127.0.0.1:8000/mcp
```

An empty body returns `501 Not Implemented` to show that the shell is reachable but not a full unrestricted tool runtime.

The staged tool-discovery contract exposes `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file`:

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp
```

Expected discovery shape for this stage: seven tools named `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file`; no Android control, shell fallback, command-capable, arbitrary service inspection, service mutation/control, or high-impact tools.

Call the read-only status tool:

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"runtime_status","arguments":{}}}' \
  http://127.0.0.1:8000/mcp
```

Call the read-only project service status tool for the allowlisted logical runtime service:

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"project_service_status","arguments":{"service_name":"mcp_runtime"}}}' \
  http://127.0.0.1:8000/mcp
```

Expected behavior: the response returns structured read-only status for the allowlisted project-owned logical service and rejects non-allowlisted service names as invalid params.

List a safe-rooted directory without reading file contents:

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_directory","arguments":{"path":"/data/data/com.termux/files/home/mcp-files","max_depth":1}}}' \
  http://127.0.0.1:8000/mcp
```

Expected behavior: the response returns metadata for entries beneath the configured safe root and rejects traversal or paths outside the safe-root boundary.

Read a bounded UTF-8 file beneath a safe root:

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"/data/data/com.termux/files/home/mcp-files/example.txt"}}}' \
  http://127.0.0.1:8000/mcp
```

Expected behavior: the response returns UTF-8 text content only for files inside configured safe roots and rejects traversal, outside-root paths, oversized files, and non-readable files.

Dry-run a safe-rooted file write:

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"write_file","arguments":{"path":"/data/data/com.termux/files/home/mcp-files/example.txt","content":"example","dry_run":true}}}' \
  http://127.0.0.1:8000/mcp
```

Expected behavior: omitted `dry_run` defaults to dry-run mode. Actual writes require explicit `dry_run:false` and still remain constrained by safe roots and payload-size limits.

## MCP Transport Restoration Gate

Do not claim broad MCP readiness until all of the following are true:

1. Every newly exposed MCP capability is restored intentionally in its own validated stage.
2. Dependency advisories for the chosen MCP stack are closed or documented with an accepted exception.
3. CI and Security workflows are green on the exact PR head, or a documented path-filtered non-run is accepted for docs-only changes.
4. Smoke tests prove MCP tool discovery and representative tool calls.
5. README, operations, security, and validation docs match the runtime behavior.

See `docs/VALIDATION.md` for repository validation expectations.
