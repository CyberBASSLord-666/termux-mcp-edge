# Termux MCP Edge (Rust)

Termux MCP Edge is currently a hardened Rust/Axum HTTP service for Android Termux deployments. The current compiled runtime exposes a health-check endpoint and enforces fail-closed authentication posture at startup.

MCP transport and MCP tool endpoints are intentionally **not compiled into the current runtime**. Earlier `rmcp`-backed transport and tool code was quarantined or removed while dependency advisories and API compatibility were being addressed. Restoring MCP transport is tracked separately and must be validated with exact-head CI, Security, and an MCP tool-list/tool-call smoke test before release.

## Current Runtime Scope

- **Runtime:** Rust single binary using Axum.
- **Current HTTP endpoint:** `GET /health`.
- **Current MCP transport:** not exposed.
- **Current filesystem/tool endpoints:** not exposed.
- **Authentication posture:** startup fails closed unless a non-empty static bearer token is configured or explicit localhost-only development mode is enabled.
- **Deployment target:** Termux on Android, supervised by `termux-services` / runit.

## Design Goals

- Memory efficiency and thermal resilience on mobile hardware.
- Fail-closed startup posture for networked deployments.
- Clear separation between current runtime behavior and future MCP transport work.
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

## Architecture

- **Language:** Rust edition 2021.
- **HTTP framework:** Axum.
- **Transport currently compiled:** health-check HTTP route only.
- **MCP framework dependency:** not compiled in the current runtime.
- **Supervision:** `termux-services` / runit.
- **Networking:** bind to localhost by default; prefer VPN or named tunnel only after authentication is configured.

## Quick Build

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo build --release
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

## MCP Transport Restoration Gate

Do not claim MCP readiness until all of the following are true:

1. A compatible MCP transport implementation is restored intentionally.
2. Dependency advisories for the chosen MCP stack are closed or documented with an accepted exception.
3. CI and Security workflows are green on the exact PR head.
4. A smoke test proves MCP tool discovery and at least one tool call.
5. README, operations, security, and validation docs match the runtime behavior.

See `docs/VALIDATION.md` for repository validation expectations.
