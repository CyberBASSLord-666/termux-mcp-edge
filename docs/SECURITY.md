# Security Best Practices for Termux MCP Edge

## Current Security Posture

Termux MCP Edge has five deliberate compile-time postures:

- The default feature set exposes the Axum `GET /health` and `GET /ready` operational endpoints and validates fail-closed startup authentication configuration.
- The optional `mcp-runtime` feature additionally exposes stable MCP 2025-11-25 Streamable HTTP handling at `/mcp` and its narrowly scoped staged tool registry.
- The optional `android-battery-status` feature includes `mcp-runtime` and permits a separately runtime-gated read-only battery tool.
- The optional `android-volume-status` feature includes `mcp-runtime` and permits a separately runtime-gated read-only audio-stream volume-status tool.
- The optional `command-execution` feature includes `mcp-runtime` and permits a separately runtime-gated fixed-profile server diagnostic tool.

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

An `android-battery-status` build may additionally expose `android_battery_status` only when `MCP__ANDROID__BATTERY_STATUS_ENABLED=true`. The runtime flag defaults to disabled and is rejected if the compile feature is absent. The provider directly executes one fixed Termux:API program with no arguments, null stdin, a cleared inherited environment, a five-second normal-operation budget with a reserved cleanup window, and hard stdout/stderr ceilings. A single cancellation-safe supervisor isolates the provider process group, terminates it immediately on overflow or cancellation, closes both pipes, and synchronously reaps the direct child. If reaping misses the reserve, the stable wait-failure result becomes authoritative and the supervisor remains responsible until collection. It returns only normalized allowlisted fields and never reflects technology/vendor strings, identifiers, raw output, stderr, paths, or environment values.

An `android-volume-status` build may additionally expose `android_volume_status` only when `MCP__ANDROID__VOLUME_STATUS_ENABLED=true`. Its runtime flag has the same compile/runtime fail-closed relationship. It directly executes only the fixed `termux-volume` path with zero arguments, so callers cannot reach the upstream command's mutation mode. The shared Android provider supervisor applies the same environment, process-group, cancellation, cleanup, and reaping guarantees with 8 KiB/4 KiB output ceilings. Parsing requires the exact six official streams and exact integer fields; it canonicalizes output order and rejects unknown, duplicate, missing, extra, or range-invalid data without reflection.

A `command-execution` build may additionally expose `run_command_profile` only when `MCP__COMMAND__ENABLED=true`. Its only profiles run the exact current server binary with project-owned argv for version, help, or boundary self-check output. The working directory is an anchored safe root; the inherited environment is empty; stdin is null; time, both output streams, concurrency, process groups, cancellation cleanup, and direct-child reaping are bounded. The request cannot select a program, argv, cwd, environment, stdin, timeout, or limit. Failures suppress child output and use stable reasons.

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
| `android_battery_status` | Disabled | Available only in the `android-battery-status` build with explicit runtime opt-in; bounded read-only telemetry |
| `android_volume_status` | Disabled | Available only in the `android-volume-status` build with explicit runtime opt-in; bounded read-only telemetry |
| `run_command_profile` | Disabled | Available only in the `command-execution` build with explicit runtime opt-in; three fixed read-only server diagnostics |
| `project_service_status` | Disabled | Read-only allowlisted project service metadata |
| `list_directory` | Disabled | Bounded and safe-rooted |
| `read_file` | Disabled | Bounded UTF-8 and safe-rooted |
| `write_file` | Disabled | Payload-bounded, safe-rooted, dry-run by default |
| Android control / arbitrary command execution / high-impact controls | Disabled | Disabled |

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

The body ceiling is implemented with Axum's streaming extractor limit rather than a second full body buffer. This keeps peak memory usage predictable on Termux. The request timeout covers body extraction and dispatch; write-side temporary-file handling is cancellation-safe and regression-tested so timed-out writes do not strand staging files.

## Filesystem and Tool Safety Rules

Filesystem requests must lexically identify a descendant of a configured safe root. The implementation rejects relative paths, NUL bytes, explicit parent traversal, missing parents, and every symlink component used by a live operation.

Each live list, read, and write opens the selected safe root and resolves descendants one component at a time with descriptor-relative `openat` plus `O_NOFOLLOW`; directory enumeration and metadata lookup remain relative to the held directory descriptor. A concurrent directory/symlink exchange can therefore produce a bounded failure or continue against the already-open safe directory capability, but cannot redirect the operation through the replacement symlink. Deterministic adversarial tests cover swaps both before and after the parent descriptor is opened.

The default safe root is deliberately narrow:

```text
/data/data/com.termux/files/home/mcp-files
```

Broad shared-storage roots such as `/storage/emulated/0` and `/sdcard` are not defaults. Empty safe-root lists or entries, relative roots, and filesystem root `/` are rejected during configuration validation. Safe-root entries are not trimmed: whitespace is path data and a value that becomes relative because of leading whitespace fails closed.

`list_directory`, `read_file`, and `write_file` are payload bounded. Directory results are deterministic, capped at 4,096 entries and a 256 KiB full response, and explicitly report truncation. Reads accept at most 1 MiB of valid UTF-8 while the full response is capped at 1,114,112 bytes; file content is emitted once rather than duplicated into the summary. `write_file` defaults to preview behavior; mutation requires explicit `dry_run:false` and still passes safe-root and payload validation. Explicit writes create an exclusive mode-0600 temporary file in the opened destination directory, sync its contents, rename relative to that same descriptor, and sync the parent directory. Cancellation before rename leaves descriptor-relative cleanup armed; after a successful rename the parent sync defines the crash-durability boundary.

Read-only metadata tools must not expose environment values, raw secrets, persistent device identifiers, global process inventories, unrelated service state, or command output.

The battery and volume providers are not general command runners. Callers cannot choose their executable, arguments, stdin, environment, timeout, output limit, or parsed fields. Disabled, unavailable, timeout, overflow, process, and parsing failures return stable reason codes without process paths, exit details, or raw output. Neither tool grants Android device-control or audio-mutation authority.

## Audit Counter Privacy

The staged runtime exposes in-memory aggregate audit counters through `runtime_status`. Counters retain stable tool names, allowed/denied totals, and low-cardinality reason codes only.

They must not retain raw paths, file contents, command arguments or output, environment names or values, bearer tokens, capability-token values, hostnames, usernames, Android identifiers, or arbitrary caller strings.

Audit counters provide evidence of gate decisions; they are not authorization and reset when the process restarts. Authentication failures are deliberately handled before MCP tool audit counters because unauthorized callers must not enter the MCP dispatch path.

## Command and High-Impact Capability Boundaries

The fixed-profile command gate is live only in its separate build and only after runtime opt-in. It authorizes three read-only diagnostics of the exact server binary, not a shell or general process launcher. Its complete boundary is documented in [`command-execution-gate.md`](command-execution-gate.md).

Capability-token primitives remain inert policy scaffolding. Any new executable, parameterized profile, mutating command, Android/service/package/network control, or other high-impact surface requires its own focused gate with compile-time and runtime opt-in, threat review, fixed allowlists, bounded execution, structured denial behavior, audit coverage, tests, and operator documentation.

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
