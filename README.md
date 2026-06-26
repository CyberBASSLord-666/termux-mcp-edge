# Termux MCP Server v5.0 (Rust) - Enterprise Edition

Production-grade Model Context Protocol server for high-end Android devices (Samsung Galaxy Z Fold 6 and similar).

## Design Goals

- Memory efficiency and thermal resilience on mobile hardware
- Zero-trust authentication
- Robust filesystem operations with symlink protection
- Single-binary deployment optimized for `termux-services` + runit
- Proper async task lifecycle management

## Security & Authentication

This server implements **zero‑trust principles** by default.  All tool calls are authenticated and sandboxed.  For simplicity the binary supports a static bearer token, but for enterprise deployments you should integrate with an OAuth 2.1 provider using PKCE (Proof Key for Code Exchange) to obtain short‑lived access tokens.  See [`docs/SECURITY.md`](docs/SECURITY.md) for a detailed discussion of threats, authentication patterns and hardening guidelines.

## Architecture

- **Language**: Rust (edition 2021)
- **MCP Framework**: `rmcp` + Axum
- **Transport**: Streamable HTTP (`stateless_http` equivalent)
- **Supervision**: `termux-services` + runit
- **Networking**: Named Cloudflare Tunnel (recommended)

## Quick Build (Cross-Compilation Recommended)

```bash
# On your development machine (Linux/macOS)
rustup target add aarch64-linux-android

# Install Android NDK if not present
# Then build:
cargo build --release --target aarch64-linux-android
```

Transfer the resulting binary from `target/aarch64-linux-android/release/termux-mcp-server` to your device.

## Samsung Galaxy Z Fold 6 Specific Setup

1. **Disable Phantom Process Killer**:
   - Enable Developer Options
   - Toggle **"Disable child process restrictions"**.  On Android 14 or newer this option stops the system from terminating background processes【48348016950568†L254-L271】.  If you disable Developer Options later, the toggle resets automatically.

2. **Disable RAM Plus**:
   - Settings → Device Care → Memory → Turn off RAM Plus

3. **Battery & Background**:
   - Set Termux to **Unrestricted** battery usage
   - Remove Termux from Deep sleeping apps
   - Disable Auto Blocker (Security and Privacy)
   - In Termux’s notification panel, tap **Acquire wakelock**.  A wakelock prevents the device from entering deep sleep so your server continues running in the background.  Only hold a wakelock while the server is in use and release it as soon as possible to preserve battery life【402006191980019†L497-L512】.

4. **Wake Lock** (in Termux):
   ```bash
   termux-wake-lock
   ```
Note that acquiring a wakelock consumes battery; avoid holding it longer than needed【402006191980019†L497-L512】.

## General Termux Setup

1. **Install `termux-services`**:
   ```bash
   pkg install termux-services
   ```
   Restart Termux so the runit supervisor starts.  All services live under `$PREFIX/var/service`【725050930974417†L52-L69】.

2. **Create a service directory and `run` script** for the server.  An example is provided in the next section.  Enable the service at boot with:
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
export MCP__FILE__SAFE_ROOTS='["/storage/emulated/0"]'
exec /data/data/com.termux/files/home/termux-mcp-server
```

Enable with:
```bash
sv-enable mcp-server
sv up mcp-server
```

### Exposing the server

For remote access we recommend running the server behind a **named Cloudflare Tunnel** instead of exposing raw ports.  Use the provided script `scripts/setup_named_tunnel.sh` to create a tunnel and route a DNS name to it.  Then update your runit `run` script to invoke:

```bash
cloudflared tunnel run <YOUR_TUNNEL_NAME> &
exec /data/data/com.termux/files/home/termux-mcp-server
```

This removes the need to open inbound ports on your device while still allowing agents to reach the service.

## Authentication

Set the `MCP__AUTH__STATIC_TOKEN` environment variable to a strong random string. All requests must include:

```
Authorization: Bearer <your-token>
```

## Health Check

```
GET /health
```

## Extending Tools

Add new tools by implementing the `#[tool]` macro in `src/tools/`. The architecture is designed for clean extension.

## Security Notes

- All file operations are strictly sandboxed to configured safe roots.
- Symlink attacks are mitigated via absolute path resolution before containment checks.
- Background tasks use proper ownership tracking to prevent silent failures.

This implementation follows the highest practical standards for a self-hosted mobile edge MCP node.
