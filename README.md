# Termux MCP Edge (Rust)

Termux MCP Edge is a hardened Rust/Axum HTTP service for Android Termux deployments. The default runtime exposes health and readiness endpoints and enforces fail-closed authentication posture at startup.

The project is designed for developers, advanced Termux operators, and power users who understand that MCP tools can affect local device state. Capabilities are introduced through explicit opt-in configuration, allowlists, bounded inputs and outputs, dry-run or preview behavior, tests, and audit coverage.

The optional `mcp-runtime` feature wires a stable MCP 2025-11-25 Streamable HTTP `/mcp` transport around the staged tool surface. In static-token mode, bearer authentication is enforced before resource-limit accounting, transport validation, JSON-RPC parsing, lifecycle handling, tool discovery, or tool invocation. Authenticated requests must pass mobile-conscious concurrency, timeout, body-size, exact `Host`, and browser `Origin` checks.

The transport negotiates protocol version `2025-11-25`, issues bounded cryptographically random sessions, requires `notifications/initialized` before normal operations, enforces media and protocol headers, accepts one JSON-RPC request, notification, or response per POST, and supports explicit session termination. GET is implemented with the specification-permitted HTTP 405 response because this server does not initiate SSE streams or retain replay state.

## Current runtime scope

- **Runtime:** Rust single binary using Axum.
- **Package version:** `0.5.1`.
- **Operational endpoints:** `GET /health` and `GET /ready`.
- **Optional MCP endpoint:** authenticated Streamable HTTP `POST`, `GET`, and `DELETE /mcp` handling when built with `--features mcp-runtime`; GET returns 405 because optional SSE delivery is not offered.
- **Staged MCP discovery:** `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `read_file`, and `write_file`.
- **Filesystem surface:** bounded safe-rooted directory listing and UTF-8 reads; safe-rooted writes are payload-bounded, cancellation-safe, and dry-run by default. Descriptor-relative race hardening remains tracked by #200.
- **Authentication:** startup fails closed unless a non-empty static token is configured or explicit localhost-only development mode is enabled.
- **Transport ordering:** authentication precedes MCP resource limits, exact Host/Origin validation, body parsing, and dispatch.
- **Mobile defaults:** four concurrent authenticated MCP requests, a 30-second request timeout, and a 2 MiB request body.
- **Session bounds:** 64 in-memory UUID sessions with a 30-minute idle expiry; client initialization metadata is validated but not retained.
- **Default filesystem root:** `/data/data/com.termux/files/home/mcp-files`.
- **Project service name:** `mcp_runtime`.
- **Deployment:** versioned Termux releases with atomic activation, health/readiness validation, and rollback.

Android platform control, shell fallback, arbitrary command execution, global process inspection, arbitrary service control, package management, network mutation, and high-impact actions remain unavailable until their separate capability gates are implemented and validated.

## Security and authentication

Set a strong token before starting a static-token deployment:

```bash
export MCP__AUTH__STATIC_TOKEN='replace-with-a-strong-random-token'
```

Every `/mcp` request must then include:

```http
Authorization: Bearer <configured-token>
```

Missing, malformed, oversized, or incorrect credentials receive HTTP 401 before MCP resource consumption or discovery. `/health` and `/ready` remain unauthenticated coarse operational probes.

## MCP transport contract

Every POST must use `Content-Type: application/json` and explicitly accept both response media types:

```http
Accept: application/json, text/event-stream
```

Start a session with an `initialize` request containing `protocolVersion`, `capabilities`, and `clientInfo`. The response negotiates `2025-11-25` and returns `MCP-Session-Id`. Every later POST, GET, or DELETE must include both:

```http
MCP-Protocol-Version: 2025-11-25
MCP-Session-Id: <value returned by initialize>
```

Send `notifications/initialized` before discovery or invocation. Accepted notifications and client responses return HTTP 202 with no body. A valid GET with `Accept: text/event-stream` returns HTTP 405 because server-initiated SSE and resumption are not offered. DELETE terminates the session with HTTP 204; expired, terminated, or unknown session IDs return HTTP 404. Session IDs scope lifecycle state but never replace bearer authentication.

Local unauthenticated development requires both:

```bash
export MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true
export MCP__SERVER__HOST=localhost
```

This mode is rejected for non-loopback binds and must not be combined with tunnels, LAN exposure, or reverse proxies.

Only absent configuration variables use defaults. Every present security- or network-relevant value must be valid Unicode, and the listener setting

```text
MCP__SERVER__PORT=8000
```

must be an integer from `1` through `65535`; port `0` is rejected because supervised deployments require a stable listener. Comma-separated safe roots and transport allowlists reject empty entries and preserve each entry exactly rather than trimming it.

Exact transport allowlists use comma-separated values:

```bash
export MCP__TRANSPORT__ALLOWED_HOSTS='localhost:8000,127.0.0.1:8000'
export MCP__TRANSPORT__ALLOWED_ORIGINS='http://localhost:8000,http://127.0.0.1:8000'
```

`MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=true` is only for reviewed non-browser clients that cannot send an `Origin` header.

## MCP request resource limits

| Setting | Default | Valid range |
|---|---:|---:|
| `MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS` | `4` | `1–64` |
| `MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS` | `30` | `1–300` |
| `MCP__TRANSPORT__MAX_BODY_BYTES` | `2097152` | `1024–8388608` |

Unsafe values fail startup validation. Saturation returns HTTP 503 with `Retry-After: 1`; timeout returns HTTP 504; oversized bodies return HTTP 413. Limit failures use non-sensitive JSON and `Cache-Control: no-store`.

Authentication is the outer gate, so unauthenticated traffic does not consume MCP concurrency permits or body-buffer capacity. `/ready` reports the active non-sensitive limit values when `mcp-runtime` is enabled.

## Filesystem safe roots

The service does not default to broad Android shared storage. Keep `MCP__FILE__SAFE_ROOTS` limited to dedicated project directories. Empty root lists or entries, relative roots, filesystem root `/`, traversal, and static symlink escapes are rejected. These checks do not yet close every canonicalize-then-use race.

## Build and validate

Run the exact CI gates:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

Build both supported postures:

```bash
cargo build --release
cargo build --release --features mcp-runtime
```

The binary exposes deployment-facing metadata without requiring runtime configuration:

```bash
./target/release/termux-mcp-server --version
./target/release/termux-mcp-server --help
```

## Termux deployment

Use [`docs/TERMUX_DEPLOYMENT.md`](docs/TERMUX_DEPLOYMENT.md) as the canonical install, upgrade, rollback, recovery, status, and uninstall path. The deployment manager:

- validates artifact checksum, architecture, executable state, size, and embedded version;
- keeps configuration outside versioned releases;
- serializes mutations with a project lock;
- creates only the fixed `mcp_runtime` runit service;
- activates releases atomically;
- restores prior release links, restarts the prior active runtime, and re-probes it when candidate or rollback validation fails.

Use [`docs/operator-validation.md`](docs/operator-validation.md) for authenticated MCP, audit-counter, filesystem, Android-status, service-status, and capability-boundary checks.

## Project documentation

- [Operations guide](docs/OPERATIONS.md)
- [Security guide](docs/SECURITY.md)
- [Validation guide](docs/VALIDATION.md)
- [Production readiness checklist](docs/PRODUCTION_READINESS.md)
- [Transport threat model](docs/TRANSPORT_THREAT_MODEL.md)
- [MCP runtime validation plan](docs/MCP_RESTORATION_VALIDATION.md)
- [MCP runtime roadmap](docs/MCP_RUNTIME_ROADMAP.md)
- [Android artifact contract](docs/ANDROID_ARTIFACTS.md)
- [Termux deployment and recovery](docs/TERMUX_DEPLOYMENT.md)
- [Operator validation checklist](docs/operator-validation.md)

## Architecture

- Rust 2021 single binary.
- Axum HTTP runtime.
- Minimal internal stable MCP 2025-11-25 Streamable HTTP transport; no external MCP framework dependency and no optional SSE/replay subsystem.
- `termux-services` / runit supervision.
- Localhost-first networking with explicit authenticated remote-access posture.
- Staged capability gates for higher-risk developer and power-user functionality.
