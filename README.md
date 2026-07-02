# Termux MCP Edge (Rust)

Termux MCP Edge is a hardened Rust/Axum HTTP service for Android Termux deployments. The default runtime exposes a simple health‑check endpoint and enforces a fail‑closed authentication posture at startup.

With the optional `mcp-runtime` feature enabled, the service wires a staged `/mcp` transport shell that validates `Host` and `Origin` headers before handling requests. The staged shell supports a JSON‑RPC discovery contract with `initialize`, `tools/list`, deterministic read‑only `runtime_status`, a safe‑rooted `list_directory` tool, safe‑rooted `read_file` and `write_file` tools (writes default to dry‑run unless explicitly disabled), and a basic `platform_info` tool. Android platform tools, command execution, and other high‑impact actions remain unavailable until separate, independently validated PRs restore them.

## Current Runtime Scope

- **Runtime:** single Rust binary using Axum.
- **Default HTTP endpoint:** `GET /health` returns `ok` when the server is reachable.
- **Optional MCP transport shell:** `POST /mcp` when compiled with `--features mcp-runtime`. Transport security validation applies to every request.
- **Current MCP discovery:** `initialize` plus `tools/list` returning `runtime_status`, `list_directory`, `read_file`, `write_file`, and `platform_info`.
- **Current MCP tools:**
  - `runtime_status` — deterministic read‑only runtime metadata.
  - `list_directory` — bounded safe‑rooted directory listing.
  - `read_file` — read the contents of a file within a configured safe root.
  - `write_file` — write data to a file within a configured safe root. Writes default to dry‑run and must be explicitly requested to persist.
  - `platform_info` — return basic host platform information such as operating system and architecture.
- **Filesystem/tool endpoints:** safe‑rooted read and write operations. Writes may be validated without persisting data by setting `dry_run=true`.
- **Authentication posture:** startup fails closed unless a non‑empty static bearer token is configured or explicit localhost‑only development mode is enabled.
- **Transport posture:** configured exact `Host` and browser `Origin` allow‑lists are enforced before the MCP transport shell handles requests.
- **Filesystem safe‑root default:** `/data/data/com.termux/files/home/mcp-files`, not broad shared storage.
- **Deployment target:** Termux on Android, supervised by `termux-services` / runit.

## Design Goals

- Memory efficiency and thermal resilience on mobile hardware.
- Fail‑closed startup posture for networked deployments.
- Clear separation between transport liveness, tool discovery, low‑risk read‑only tools, filesystem tools, platform tools, and later high‑impact tooling.
- Narrow default filesystem scope for any file‑capable tool.
- Single‑binary deployment optimized for `termux-services` and runit.
- CI and Security workflows as merge gates for every remediation branch.

## Security and Authentication

Set `MCP__AUTH__STATIC_TOKEN` to a strong random value before starting the service. Empty or whitespace‑only values are rejected at startup.

Local unauthenticated development requires both conditions:

```bash
export MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true
export MCP__SERVER__HOST=localhost
```

This opt‑in is rejected for non‑loopback bind addresses and must not be used with tunnels, LAN exposure, reverse proxies, or shared network access.

Browser‑reachable MCP transport requests are additionally constrained by exact transport allow‑lists:

```bash
export MCP__TRANSPORT__ALLOWED_HOSTS='["localhost:8000"]'
export MCP__TRANSPORT__ALLOWED_ORIGINS='["http://localhost:8000"]'
```

`MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=true` is only for explicitly reviewed non‑browser clients that cannot send an `Origin` header.

## Filesystem Safe Roots

The built‑in filesystem safe‑root default is intentionally narrow:

```text
/data/data/com.termux/files/home/mcp-files
```

The service no longer defaults to broad Android shared‑storage roots such as `/storage/emulated/0` or `/sdcard`. File I/O tools enforce that every requested path resolves within one of the configured safe roots. Operators should keep `MCP__FILE__SAFE_ROOTS` limited to a dedicated project directory and avoid granting all shared storage unless there is a specific reviewed need.

## Architecture

- **Language:** Rust edition 2021.
- **HTTP framework:** Axum.
- **Default compiled transport:** health‑check HTTP route only.
- **Optional MCP transport shell:** feature‑gated `/mcp` route with transport security validation, `initialize`, `tools/list`, `runtime_status`, `list_directory`, `read_file`, `write_file`, and `platform_info`.
- **MCP framework dependency:** optional `rmcp` dependency behind `mcp-runtime`.
- **Supervision:** `termux-services` / runit.
- **Networking:** bind to localhost by default; prefer VPN or named tunnel only after authentication is configured.

## Runtime Roadmap

MCP runtime restoration is staged in [`docs/MCP_RUNTIME_ROADMAP.md`](docs/MCP_RUNTIME_ROADMAP.md). The roadmap keeps transport restoration, tool discovery, read‑only tools, filesystem tools, platform tools, and high‑impact tools in separate validation tracks.

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

For Android cross‑compilation:

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

Create a local bearer‑token file before enabling the runit service:

```bash
umask 077
openssl rand -hex 32 > "$HOME/.termux_mcp_token"
chmod 600 "$HOME/.termux_mcp_token"
```

The packaged runit script fails before starting the service if the token file is missing, empty, or whitespace‑only.

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

An empty body returns `501 Not Implemented` to show that the shell is reachable but not a full unrestricted tool runtime.

The staged tool‑discovery contract exposes five tools in this stage: `runtime_status`, `list_directory`, `read_file`, `write_file`, and `platform_info`:

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp
```

### Reading a file

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"/data/data/com.termux/files/home/mcp-files/visible.txt"}}}' \
  http://127.0.0.1:8000/mcp
```

### Writing a file (dry‑run)

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"write_file","arguments":{"path":"/data/data/com.termux/files/home/mcp-files/new.txt","content":"hello world","dry_run":true}}}' \
  http://127.0.0.1:8000/mcp
```

### Retrieving platform information

```bash
curl -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"platform_info","arguments":{}}}' \
  http://127.0.0.1:8000/mcp
```

Expected behaviour: the response returns the OS, architecture and family of the host; no Android APIs are called.

## MCP Transport Restoration Gate

Do not claim MCP readiness until all of the following are true:

1. A compatible MCP transport implementation is restored intentionally.
2. Dependency advisories for the chosen MCP stack are closed or documented with an accepted exception.
3. CI and Security workflows are green on the exact PR head.
4. A smoke test proves MCP tool discovery and at least one tool call.
5. README, operations, security, and validation docs match the runtime behaviour.

See `docs/VALIDATION.md` for repository validation expectations.
