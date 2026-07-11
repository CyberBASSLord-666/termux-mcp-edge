# Security Best Practices for Termux MCP Edge

## Current Security Posture

Termux MCP Edge has two deliberate compile-time postures:

- The default feature set exposes the Axum `GET /health` and `GET /ready` operational endpoints and validates fail-closed startup authentication configuration.
- The optional `mcp-runtime` feature additionally exposes stable MCP 2025-11-25 Streamable HTTP handling at `/mcp` and its narrowly scoped staged tool registry.

The transport negotiates protocol version `2025-11-25`, requires initialization before normal operations, enforces JSON/SSE media acceptance and subsequent protocol/session headers, and uses bounded in-memory UUID sessions. GET returns the specification-permitted HTTP 405 because optional server-initiated SSE, replay, and resumption are not implemented.

In static-token mode, the complete `/mcp` route requires `Authorization: Bearer <configured-token>` before request resource limits, transport validation, JSON-RPC parsing, tool discovery, or tool invocation. Missing, malformed, oversized, or incorrect credentials are rejected with HTTP 401 and a non-sensitive response. The only authentication bypass is explicit unauthenticated localhost-only development mode, which startup validation restricts to a loopback bind.

The optional runtime is not a broad host-control surface. After authentication, it enforces bounded concurrency, request duration, and request-body size, then validates exact `Host` and browser `Origin` allowlists before dispatch. It exposes only the currently documented staged tools:

- `runtime_status`
- `platform_info`
- `android_status`
- `project_service_status`
- `list_directory`
- `read_file`
- `write_file`

Android platform control, shell fallback, arbitrary command execution, global process inventory, arbitrary service inspection, service mutation/control, package management, network mutation, and other high-impact controls remain disabled.

## Authentication and Startup Behavior

Startup requires `MCP__AUTH__STATIC_TOKEN` by default. Empty or whitespace-only values are rejected before the HTTP listener starts. The configured token is redacted from debug output and must not be logged or copied into issue reports.

Only an absent environment variable may select its documented default. Present non-Unicode values for the bearer token, listener host/port, safe roots, transport allowlists, compatibility switch, or request limits fail startup with non-sensitive errors. `MCP__SERVER__PORT` accepts only `1–65535`; port `0` is not a supported supervised-listener configuration.

The only supported exception is explicit local development mode:

```bash
export MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true
export MCP__SERVER__HOST=127.0.0.1
```

This opt-in is rejected for non-loopback bind addresses. Do not use unauthenticated mode with tunnels, LAN exposure, reverse proxies, shared devices, or any remotely reachable deployment.

For a static-token deployment, every MCP request must include:

```http
Authorization: Bearer <configured-token>
```

The bearer scheme is case-insensitive, but the token value is an exact match. Authentication failures include `WWW-Authenticate: Bearer` and `Cache-Control: no-store` and must not reveal whether a token was missing, malformed, or incorrect.

## Current Endpoint and Tool Surface

| Surface | Default build | `mcp-runtime` build |
|---|---:|---:|
| `GET /health` | Enabled, unauthenticated | Enabled, unauthenticated |
| `GET /ready` | Enabled, unauthenticated | Enabled, unauthenticated with non-sensitive limit metadata |
| `POST`, `GET`, `DELETE /mcp` stable transport | Disabled | Bearer-authenticated, resource-bounded, except explicit loopback development mode; GET returns 405 without SSE |
| `runtime_status` / `platform_info` | Disabled | Read-only |
| `android_status` | Disabled | Read-only allowlisted metadata |
| `project_service_status` | Disabled | Read-only allowlisted project service metadata |
| `list_directory` | Disabled | Bounded and safe-rooted |
| `read_file` | Disabled | Bounded UTF-8 and safe-rooted |
| `write_file` | Disabled | Payload-bounded, safe-rooted, dry-run by default |
| Android control / command execution / high-impact controls | Disabled | Disabled |

The unauthenticated operational endpoints are intentionally coarse. They must not return secrets, raw configuration, private paths, tool discovery, or tool results.

## Transport Security

Browser-reachable MCP requests must match the configured exact transport allowlists after authentication and request-limit admission succeed:

```bash
export MCP__TRANSPORT__ALLOWED_HOSTS='localhost:8000'
export MCP__TRANSPORT__ALLOWED_ORIGINS='http://localhost:8000'
```

`MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=true` is only appropriate for explicitly reviewed non-browser clients that cannot send `Origin`. It must not be used as a general browser compatibility bypass.

Authentication occurs before request-limit accounting, transport validation, and JSON-RPC dispatch. Rejected credentials must not consume MCP concurrency permits or body-buffer capacity. Authenticated requests with rejected hosts or origins must not reach JSON-RPC or tool-call handling.

Every POST must use `Content-Type: application/json` and explicitly accept both `application/json` and `text/event-stream`. After initialize returns `MCP-Session-Id`, every subsequent POST, GET, or DELETE must send that identifier and `MCP-Protocol-Version: 2025-11-25`. Duplicate, malformed, missing, unknown, expired, and mismatched header states fail closed. Accepted notifications and client responses return HTTP 202 without bodies.

Session identifiers are UUID v4 values, but they are not authentication credentials. The server re-runs bearer authentication for every request before session lookup. The in-memory store retains no client-provided identity or capability metadata, holds at most 64 sessions, expires idle records after 30 minutes, and supports explicit DELETE termination. Protect session identifiers from disclosure anyway: a caller possessing both the shared operator bearer token and an active session ID can act within that session. All `/mcp` handler responses use `Cache-Control: no-store`.

## Request Resource Limits

The MCP transport uses explicit limits intended for a supervised mobile process:

- `MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS`: default `4`, valid `1–64`.
- `MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS`: default `30`, valid `1–300`.
- `MCP__TRANSPORT__MAX_BODY_BYTES`: default `2097152`, valid `1024–8388608`.

Values outside these ranges fail startup validation. Concurrency saturation fails fast with HTTP 503 and `Retry-After: 1`. Request timeout returns HTTP 504. Request bodies over the configured ceiling return HTTP 413. All limit responses use non-sensitive JSON and `Cache-Control: no-store`.

The body ceiling is implemented with Axum's streaming extractor limit rather than a second full body buffer. This keeps peak memory usage predictable on Termux. The request timeout covers body extraction and dispatch; write-side temporary-file handling must remain cancellation-safe before timeout enforcement is considered production-ready.

## Filesystem and Tool Safety Rules

Filesystem paths are canonicalized or resolved through existing parents and must remain inside configured safe roots. The implementation rejects relative paths, NUL bytes, explicit parent traversal, missing unsafe parents, and symlink escapes beyond a safe root.

These checks constrain static path escapes but do not close every canonicalize-then-use race. Descriptor-relative operations and race-focused tests remain required before treating the filesystem surface as hardened against concurrent symlink replacement.

The default safe root is deliberately narrow:

```text
/data/data/com.termux/files/home/mcp-files
```

Broad shared-storage roots such as `/storage/emulated/0` and `/sdcard` are not defaults. Empty safe-root lists or entries, relative roots, and filesystem root `/` are rejected during configuration validation. Safe-root entries are not trimmed: whitespace is path data and a value that becomes relative because of leading whitespace fails closed.

`list_directory`, `read_file`, and `write_file` are payload bounded. Directory results are deterministic, capped at 4,096 entries and a 256 KiB full response, and explicitly report truncation. Reads accept at most 1 MiB of valid UTF-8 while the full response is capped at 1,114,112 bytes; file content is emitted once rather than duplicated into the summary. `write_file` defaults to preview behavior; mutation requires explicit `dry_run:false` and still passes safe-root and payload validation.

Read-only metadata tools must not expose environment values, raw secrets, persistent device identifiers, global process inventories, unrelated service state, or command output.

## Audit Counter Privacy

The staged runtime exposes in-memory aggregate audit counters through `runtime_status`. Counters retain stable tool names, allowed/denied totals, and low-cardinality reason codes only.

They must not retain raw paths, file contents, command arguments or output, environment names or values, bearer tokens, capability-token values, hostnames, usernames, Android identifiers, or arbitrary caller strings.

Audit counters provide evidence of gate decisions; they are not authorization and reset when the process restarts. Authentication failures are deliberately handled before MCP tool audit counters because unauthorized callers must not enter the MCP dispatch path.

## Command and High-Impact Capability Boundaries

Command-policy and capability-token modules are inert policy scaffolding. Their presence does not enable process spawning, shell access, Android control, package/service/network mutation, or any high-impact MCP tool.

Any future command-capable or high-impact surface requires its own focused gate with compile-time and runtime opt-in, fixed allowlists, bounded execution, structured denial behavior, audit coverage, tests, and operator documentation.

## Dependency Advisory Policy

Dependency advisories must be resolved by one of these paths:

1. Remove unused vulnerable dependencies.
2. Upgrade to a patched compatible version.
3. Quarantine the affected feature from the compiled target while documenting the limitation.
4. Record an explicit accepted-risk exception only when there is no safe alternative.

Cargo, lockfile, or security-workflow changes must remain separate from unrelated runtime behavior changes and require exact-head CI and Security validation before merge.

## Deployment Hardening

- Bind to localhost unless a remote access path is explicitly required.
- Configure a strong bearer token before using tunnels or LAN access.
- Prefer a VPN-bound endpoint or named tunnel over raw port exposure.
- Treat tunnel login, creation, and DNS routing as explicit operator mutations. Use `scripts/setup_named_tunnel.sh --dry-run` first; never enable DNS overwrite to bypass a route conflict.
- Keep exact Host and Origin allowlists minimal.
- Keep the mobile-conscious request-limit defaults unless measured workload requires a reviewed increase.
- Keep filesystem safe roots limited to dedicated project directories.
- Protect `$HOME/.config/termux-mcp-edge/runtime.env` with mode `0600` and avoid printing the token during validation.
- Rotate tokens after suspected exposure.
- Keep CI, Security, Dependabot, and pinned GitHub Actions enabled.
- Validate unauthorized rejection, initialization/session behavior, request-limit failures, and authenticated MCP behavior before enabling the optional runtime in a supervised service.

## Incident Response

If compromise or resource exhaustion is suspected:

1. Stop the runit service.
2. Rotate bearer tokens and tunnel credentials when credential exposure is possible.
3. Inspect service and tunnel logs without copying secrets into issues or audit counters.
4. Recheck authentication mode, request-limit configuration, transport allowlists, and filesystem safe-root configuration.
5. Review recent dependency, workflow, and runtime changes.
6. Restore conservative request-limit defaults if custom values increased memory or concurrency pressure.
7. Redeploy only after the relevant exact-head CI and Security checks are green.
