# Termux MCP Edge (Rust)

Production-grade Model Context Protocol server for high-end Android devices, especially Termux deployments on Samsung Galaxy Z Fold-class hardware.

## Design Goals

- Memory efficiency and thermal resilience on mobile hardware
- Zero-trust authentication
- Robust filesystem operations with symlink protection
- Single-binary deployment optimized for `termux-services` + runit
- Proper async task lifecycle management

## Security & Authentication

This server implements **zero-trust principles** by default. All tool calls are authenticated and sandboxed. For simplicity the binary supports a static bearer token, but for enterprise deployments you should integrate with an OAuth 2.1 provider using PKCE (Proof Key for Code Exchange) to obtain short-lived access tokens. See [`docs/SECURITY.md`](docs/SECURITY.md) for a detailed discussion of threats, authentication patterns and hardening guidelines.

Startup now fails closed when no bearer token is configured. Empty or whitespace-only bearer tokens are rejected. Local unauthenticated development requires the explicit `MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true` opt-in and a localhost bind address (`localhost`, `127.0.0.1`, or `::1`). Do not enable this mode for remotely exposed, tunneled, LAN-accessible, or rish-capable deployments.

## Architecture

- **Language**: Rust edition 2021
- **MCP Framework**: `rmcp` + Axum
- **Transport**: Streamable HTTP (`stateless_http` equivalent)
- **Supervision**: `termux-services` + runit
- **Networking**: Named Cloudflare Tunnel or VPN-bound endpoint recommended

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

Transfer the resulting binary from `target/aarch64-linux-android/release/termux-mcp-server` to your device.

See [`docs/VALIDATION.md`](docs/VALIDATION.md) for validation expectations and known limits of automated improvement runs.

## Samsung Galaxy Z Fold 6 Specific Setup

1. **Disable Phantom Process Killer**:
   - Enable Developer Options
   - Toggle **"Disable child process restrictions"**. On Android 14 or newer this option stops the system from terminating background processes【48348016950568†L254-L271】. If you disable Developer Options later, the toggle resets automatically.

2. **Disable RAM Plus**:
   - Settings → Device Care → Memory → Turn off RAM Plus

3. **Battery & Background**:
   - Set Termux to **Unrestricted** battery usage
   - Remove Termux from Deep sleeping apps
   - Disable Auto Blocker (Security and Privacy)
   - In Termux’s notification panel, tap **Acquire wakelock**. A wakelock prevents the device from entering deep sleep so your server continues running in the background. Only hold a wakelock while the server is in use and release it as soon as possible to preserve battery life【402006191980019†L497-L512】.

4. **Wake Lock** in Termux:
   ```bash
   termux-wake-lock
   ```
   Note that acquiring a wakelock consumes battery; avoid holding it longer than needed【402006191980019†L497-L512】.

## General Termux Setup

1. **Install `termux-services`**:
   ```bash
   pkg install termux-services
   ```
   Restart Termux so the runit supervisor starts. All services live under `$PREFIX/var/service`【725050930974417†L52-L69】.

2. **Create a service directory and `run` script** for the server. An example is provided in the next section. Enable the service with:
   ```bash
   sv-enable <service>
   sv up <service>
   ```
   These commands integrate with runit to supervise your server【725050930974417†L52-L109】.

## Running with Supervision

Create runit service at `$PREFIX/var/service/mcp-server/run`:

```bash
#!/data/data/com.termux/files/usr/bin/sh
exec 2>&1
export MCP__AUTH__STATIC_TOKEN="your-secure-token-here"
export MCP__FILE__SAFE_ROOTS='["/storage/emulated/0/Documents"]'
exec /data/data/com.termux/files/home/termux-mcp-server
```

Enable with:

```bash
sv-enable mcp-server
sv up mcp-server
```

## Filesystem safety model

Filesystem tools only operate on absolute paths that resolve under configured safe roots. Keep `MCP__FILE__SAFE_ROOTS` narrow, prefer a dedicated project directory, and use dry-run writes before modifying important files.

Directory listing is bounded by depth and entry count to protect latency, memory, and battery on mobile hardware.

## Exposing the server

For remote access we recommend running the server behind a **named Cloudflare Tunnel** instead of exposing raw ports. Use the provided script `scripts/setup_named_tunnel.sh` to create a tunnel and route a DNS name to it. Then update your runit `run` script to invoke:

```bash
cloudflared tunnel run <YOUR_TUNNEL_NAME> &
exec /data/data/com.termux/files/home/termux-mcp-server
```

This removes the need to open inbound ports on your device while still allowing trusted agents to reach the service. Keep `MCP__AUTH__STATIC_TOKEN` configured whenever any tunnel, VPN, LAN, reverse proxy, or non-loopback listener can reach the server.

## Authentication

Set the `MCP__AUTH__STATIC_TOKEN` environment variable to a strong random string. Empty or whitespace-only token values are rejected at startup. All requests must include:

```http
Authorization: Bearer <your-token>
```

For local development only, an unauthenticated listener can be started by setting:

```bash
export MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true
export MCP__SERVER__HOST=localhost
```

This opt-in is rejected for non-loopback bind addresses and must not be used with remote transports, tunnels, shared networks, or rish-capable tool exposure.

## Health Check

```http
GET /health
```
