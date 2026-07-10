# Termux MCP Edge (Rust)

Termux MCP Edge is a hardened Rust/Axum HTTP service for Android Termux deployments. The default runtime exposes a health-check endpoint and enforces fail-closed authentication posture at startup.

The project is designed for developers, advanced Termux operators, and power users who understand that MCP tools can affect local device state. The staged security model is not intended to remove powerful capabilities permanently; it is intended to make each capability explicit, opt-in, reviewable, testable, and auditable before it is exposed.

The optional `mcp-runtime` feature wires a staged `/mcp` transport that validates `Host` and browser `Origin` headers before handling requests. It currently supports `initialize`, `tools/list`, deterministic read-only `runtime_status`, non-sensitive read-only `platform_info`, read-only allowlisted `android_status`, safe-rooted `list_directory`, bounded safe-rooted UTF-8 `read_file`, default-dry-run safe-rooted `write_file`, and read-only allowlisted `project_service_status`. Android platform control, shell fallback, arbitrary command execution, process inventory, arbitrary service inspection, service mutation/control, and high-impact actions remain unavailable until later staged PRs validate each surface independently.

## Current Runtime Scope

- **Runtime:** Rust single binary using Axum.
- **Current package version:** `0.5.1`.
- **Default HTTP endpoint:** `GET /health`.
- **Optional MCP transport shell:** `POST /mcp` when built with `--features mcp-runtime`.
- **Current MCP discovery:** `initialize` plus `tools/list` returning `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file`.
- **Current MCP tools:** deterministic read-only runtime metadata, non-sensitive platform metadata, read-only allowlisted Android/Termux status metadata, read-only allowlisted project-owned service status metadata, bounded safe-rooted directory listing, bounded safe-rooted UTF-8 file reads, and default-dry-run safe-rooted file writes.
- **Current filesystem/tool endpoints:** directory listing and file reads are bounded to configured safe roots; `write_file` is exposed with explicit safe-root, payload-size, and dry-run-by-default controls.
- **Authentication posture:** startup fails closed unless a non-empty static bearer token is configured or explicit localhost-only development mode is enabled.
- **Transport posture:** configured exact `Host` and browser `Origin` allowlists are enforced before the staged MCP transport handles requests.
- **Filesystem safe-root default:** `/data/data/com.termux/files/home/mcp-files`, not broad shared storage.
- **Project service status scope:** `project_service_status` reports only project-owned logical services from the explicit allowlist; the current public service name is `mcp_runtime`.
- **Deployment target:** Termux on Android, supervised by `termux-services` / runit.

## Design Goals

- Serve advanced local development and power-user automation workflows without pretending high-impact MCP capabilities are risk-free.
- Memory efficiency and thermal resilience on mobile hardware.
- Fail-closed startup posture for networked deployments.
- Clear separation between transport liveness, tool discovery, low-risk read-only tools, filesystem listing, file reads, file writes, project-owned service status, and later higher-impact tool execution.
- Narrow default filesystem scope for file-capable tools.
- Explicit opt-in, allowlist, dry-run, and audit boundaries for riskier tools rather than broad always-on exposure.
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

Browser-reachable MCP transport requests are additionally constrained by exact transport allowlists:

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

## Operator Validation

Use [`docs/operator-validation.md`](docs/operator-validation.md) when validating a local build, configuration change, release candidate, manual dispatch build, or tag-triggered artifact. The checklist cross-links runtime discovery, `runtime_status` audit counters, filesystem safe-root behavior, read-only Android status, project service status, and future capability-token boundaries without enabling any new runtime surface.

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

## Validation and Build

Run the same Rust validation gates enforced by CI:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

Build the default health-only runtime:

```bash
cargo build --release
```

Build the staged MCP runtime:

```bash
cargo build --release --features mcp-runtime
```

For Android cross-compilation and operator smoke tests, follow [`docs/VALIDATION.md`](docs/VALIDATION.md).
