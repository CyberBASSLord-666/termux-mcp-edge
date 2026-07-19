# Security Best Practices for Termux MCP Edge

## Current Security Posture

Termux MCP Edge has six deliberate compile-time postures:

- The default feature set exposes the Axum `GET /health` and `GET /ready` operational endpoints and validates fail-closed startup authentication configuration.
- The optional `mcp-runtime` feature additionally exposes stable MCP 2025-11-25 Streamable HTTP handling at `/mcp` and its narrowly scoped staged tool registry.
- The optional `android-battery-status` feature includes `mcp-runtime` and permits a separately runtime-gated read-only battery tool.
- The optional `android-volume-status` feature includes `mcp-runtime` and permits a separately runtime-gated read-only audio-stream volume-status tool.
- The optional `android-volume-control` feature permits only separately runtime-gated, preview-first, exact-stream volume mutation with a fresh live bound and one exact single-use request grant.
- The optional `command-execution` feature includes `mcp-runtime` and permits a separately runtime-gated fixed-profile server diagnostic tool.

The transport negotiates protocol version `2025-11-25`, requires initialization before normal operations, enforces JSON/SSE media acceptance and subsequent protocol/session headers, and uses bounded in-memory UUID sessions. GET returns the specification-permitted HTTP 405 because optional server-initiated SSE, replay, and resumption are not implemented.

In static-token mode, the complete `/mcp` route requires `Authorization: Bearer <configured-token>` before request resource limits, transport validation, JSON-RPC parsing, tool discovery, or tool invocation. Missing, malformed, oversized, or incorrect credentials are rejected with HTTP 401 and a non-sensitive response. The only authentication bypass is explicit unauthenticated localhost-only development mode, which startup validation restricts to a loopback bind.

The optional runtime is not a broad host-control surface. After authentication, it enforces bounded concurrency, request duration, and request-body size, then validates exact `Host` and browser `Origin` allowlists before dispatch. It exposes only the currently documented staged tools:

- `runtime_status`
- `platform_info`
- `android_status`
- `project_service_status`
- `create_directory`
- `copy_file`
- `hash_file`
- `list_directory`
- `path_metadata`
- `read_file`
- `search_text`
- `write_file`

An `android-battery-status` build may additionally expose `android_battery_status` only when `MCP__ANDROID__BATTERY_STATUS_ENABLED=true`. The runtime flag defaults to disabled and is rejected if the compile feature is absent. The provider directly executes one fixed Termux:API program with no arguments, null stdin, a cleared inherited environment, a five-second normal-operation budget with a reserved cleanup window, and hard stdout/stderr ceilings. A single cancellation-safe supervisor isolates the provider process group, terminates it immediately on overflow or cancellation, closes both pipes, and synchronously reaps the direct child. If reaping misses the reserve, the stable wait-failure result becomes authoritative and the supervisor remains responsible until collection. It returns only normalized allowlisted fields and never reflects technology/vendor strings, identifiers, raw output, stderr, paths, or environment values.

An `android-volume-status` build may additionally expose `android_volume_status` only when `MCP__ANDROID__VOLUME_STATUS_ENABLED=true`. Its runtime flag has the same compile/runtime fail-closed relationship. It directly executes only the fixed `termux-volume` path with zero arguments, so callers cannot reach the upstream command's mutation mode. The shared Android provider supervisor applies the same environment, process-group, cancellation, cleanup, and reaping guarantees with 8 KiB/4 KiB output ceilings. Parsing requires the exact six official streams and exact integer fields; it canonicalizes output order and rejects unknown, duplicate, missing, extra, or range-invalid data without reflection.

An `android-volume-control` build may expose `set_android_volume` only when `MCP__ANDROID__VOLUME_CONTROL_ENABLED=true`, static-token authentication, and a complete capability key pair are active. Preview is the default and reads fresh strict status without invoking the setter. Live mutation requires one 60-second grant bound to the keyed principal, canonical session, capability, exact stream, exact level, and mutating posture. The grant is consumed immediately before fixed two-argument execution and remains consumed. One non-queueing permit rejects conflicts; fresh status verifies success; setter or verification failure triggers restoration to the captured prior level. An owned worker continues verification/recovery after caller cancellation. See [`ANDROID_VOLUME_CONTROL.md`](ANDROID_VOLUME_CONTROL.md).

A `command-execution` build may additionally expose `run_command_profile` only when `MCP__COMMAND__ENABLED=true`. Its only profiles run the exact current server binary with project-owned argv for version, help, or boundary self-check output. The working directory is an anchored safe root; the inherited environment is empty; stdin is null; time, both output streams, concurrency, process groups, cancellation cleanup, and direct-child reaping are bounded. The request cannot select a program, argv, cwd, environment, stdin, timeout, or limit. Failures suppress child output and use stable reasons.

Android controls beyond the request-authorized exact-stream volume capability, shell fallback, arbitrary command execution, global process inventory, arbitrary service inspection, service mutation/control, package management, network mutation, and unrelated high-impact controls remain disabled.

## Authentication and Startup Behavior

Startup requires `MCP__AUTH__STATIC_TOKEN` by default. Empty or whitespace-only values are rejected before the HTTP listener starts. The configured token is redacted from debug output and must not be logged or copied into issue reports.

Bearer authentication does not by itself authorize `create_directory` mutation. That mutation requires the default-disabled `MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED` gate, a paired 32-byte HMAC key configuration, and one 60-second, single-use `MCP-Capability-Grant` bound to the principal, active session, anchored safe root, normalized target, and mutating posture. The runtime consumes the JTI immediately before its first filesystem mutation attempt and retains consumption after downstream failure. Grants are header-only and must never appear in arguments, URLs, responses, logs, audit labels, tickets, or screenshots. See [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md).

Bearer authentication likewise does not authorize Android volume mutation by itself. The volume-control runtime gate and the same private key configuration are required, but its signed capability code and exact stream/level binding are distinct from directory grants. See [`ANDROID_VOLUME_CONTROL.md`](ANDROID_VOLUME_CONTROL.md).

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
| `set_android_volume` | Disabled | Available only in the `android-volume-control` build with explicit runtime opt-in; preview by default, live mutation requires one exact single-use grant and verified recovery semantics |
| `run_command_profile` | Disabled | Available only in the `command-execution` build with explicit runtime opt-in; three fixed read-only server diagnostics |
| `project_service_status` | Disabled | Read-only allowlisted project service metadata |
| `create_directory` | Disabled | Preview is available; mutation is separately default-disabled and requires fixed mode `0700`, atomic no-replace, and one request-scoped single-use grant |
| `copy_file` | Disabled | One regular file up to 1 MiB, fixed mode `0600`, atomic no-replace, content-private, dry-run by default |
| `hash_file` | Disabled | One no-follow regular file up to 16 MiB, streaming SHA-256, digest-and-size-only response |
| `list_directory` | Disabled | Bounded and safe-rooted |
| `path_metadata` | Disabled | Bounded, content-free, descriptor-relative metadata |
| `read_file` | Disabled | Bounded UTF-8 and safe-rooted |
| `search_text` | Disabled | Bounded literal locations without content excerpts |
| `write_file` | Disabled | Payload-bounded, safe-rooted, dry-run by default |
| Other Android control / arbitrary command execution / unrelated high-impact controls | Disabled | Disabled |

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

Each live create, copy, hash, list, metadata, read, search, and write opens the selected safe root and resolves descendants one component at a time with descriptor-relative `openat` plus `O_NOFOLLOW`; directory enumeration, source reads, hashing, staging, publication, cleanup, and metadata lookup remain relative to held descriptors. Copy additionally verifies source identity and size around its bounded read and verifies both held and published destination identities. Hashing verifies that the opened regular descriptor matches the pre-open no-follow device/inode observation, then enforces its byte limit while streaming. A concurrent directory/symlink exchange can therefore produce a bounded failure or continue against an already-open safe directory or file capability, but cannot redirect the operation through the replacement symlink. Deterministic adversarial tests cover swaps before and after descriptors are opened.

The default safe root is deliberately narrow:

```text
/data/data/com.termux/files/home/mcp-files
```

Broad shared-storage roots such as `/storage/emulated/0` and `/sdcard` are not defaults. Empty safe-root lists or entries, relative roots, and filesystem root `/` are rejected during configuration validation. Safe-root entries are not trimmed: whitespace is path data and a value that becomes relative because of leading whitespace fails closed.

`create_directory`, `copy_file`, `hash_file`, `list_directory`, `path_metadata`, `read_file`, `search_text`, and `write_file` are response or payload bounded. Directory creation defaults to preview; explicit `dry_run:false` selects mutation but the independent runtime gate and exact request grant still must authorize it. The runtime validates the absent descriptor-relative target before matching the grant, atomically consumes the grant immediately before `mkdirat`, publishes fixed mode `0700` without replacement, and caps the complete response at 16 KiB. File copy accepts arbitrary bytes from one regular source up to 1 MiB, requires an absent safe-rooted destination, defaults to preview, publishes mode `0600` without replacement, returns no content, and preflights its 16 KiB response. File hashing streams arbitrary bytes from one regular descriptor up to 16 MiB, returns only lowercase SHA-256 and bytes hashed, preflights its 16 KiB response before reading, and never records digest/path/content in audits. Directory listings are deterministic and response bounded. Metadata is content-free; reads accept at most 1 MiB of valid UTF-8; search returns only bounded locations. `write_file` remains dry-run first and payload bounded. Mutation cleanup is descriptor-relative and identity-checked; successful parent sync defines the crash-durability boundary.

Read-only metadata tools must not expose environment values, raw secrets, persistent device identifiers, global process inventories, unrelated service state, or command output.

The battery and volume providers are not general command runners. Callers cannot choose their executable, arguments, stdin, environment, timeout, output limit, or parsed fields. Disabled, unavailable, timeout, overflow, process, and parsing failures return stable reason codes without process paths, exit details, or raw output. Neither tool grants Android device-control or audio-mutation authority.

## Audit Counter Privacy

The staged runtime exposes in-memory aggregate audit counters through `runtime_status`. Counters retain stable tool names, allowed/denied totals, and low-cardinality reason codes only.

They must not retain raw paths, file contents, command arguments or output, environment names or values, bearer tokens, capability grants or keys, principal fingerprints, sessions, JTIs, target digests, timestamps, hostnames, usernames, Android identifiers, or arbitrary caller strings.

Audit counters provide evidence of gate decisions; they are not authorization and reset when the process restarts. Authentication failures are deliberately handled before MCP tool audit counters because unauthorized callers must not enter the MCP dispatch path.

## Command and High-Impact Capability Boundaries

The fixed-profile command gate is live only in its separate build and only after runtime opt-in. It authorizes three read-only diagnostics of the exact server binary, not a shell or general process launcher. Its complete boundary is documented in [`command-execution-gate.md`](command-execution-gate.md).

The narrowly scoped `create_directory` and Android-volume request-grant primitives are live only for their exact bound operations. They share a private key configuration and public header name but use distinct signed capability codes and target encodings, so a grant cannot cross-authorize them. The separate general-purpose capability-token policy module remains inert scaffolding. Any new executable, parameterized profile, mutating command, Android/service/package/network control, or other high-impact surface requires its own focused gate with compile-time and runtime opt-in, threat review, fixed allowlists, bounded execution, structured denial behavior, audit coverage, tests, and operator documentation.

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
