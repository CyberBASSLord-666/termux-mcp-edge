# Security Best Practices for Termux MCP Edge

## Current Security Posture

Termux MCP Edge has six deliberate compile-time postures:

- The default feature set exposes the Axum `GET /health` and `GET /ready` operational endpoints and validates fail-closed startup authentication configuration.
- The optional `mcp-runtime` feature additionally exposes stable MCP 2025-11-25 Streamable HTTP handling at `/mcp` and its narrowly scoped staged tool registry.
- The optional `android-battery-status` feature includes `mcp-runtime` and permits a separately runtime-gated read-only battery tool.
- The optional `android-volume-status` feature includes `mcp-runtime` and permits a separately runtime-gated read-only audio-stream volume-status tool.
- The optional `android-volume-control` feature permits only separately runtime-gated, preview-first, exact-stream volume mutation with a fresh live bound and one exact single-use request grant.
- The optional `command-execution` feature includes `mcp-runtime` and permits a separately runtime-gated fixed-profile server diagnostic tool.

The transport negotiates protocol version `2025-11-25`, requires initialization before normal operations, enforces JSON/SSE media acceptance and subsequent protocol/session headers, and uses bounded in-memory UUID sessions. GET returns the specification-permitted HTTP 405 by default; a separate default-disabled setting permits finite SSE response replay under fixed session-owned limits.

In static-token mode, the complete `/mcp` route requires `Authorization: Bearer <configured-token>` before request resource limits, transport validation, JSON-RPC parsing, tool discovery, or tool invocation. Missing, malformed, oversized, or incorrect credentials are rejected with HTTP 401 and a non-sensitive response. The only authentication bypass is explicit unauthenticated localhost-only development mode. Startup validates an actually bound loopback listener, and the authentication middleware separately requires request-time `ConnectInfo<McpConnectionInfo>` derived from the accepted TCP stream. Its peer must be IPv4 or IPv6 loopback and its local address must exactly match the listener validated by the builder. Missing connection metadata, non-loopback peers, and listener substitution fail closed before request limits; `Host`, `Origin`, and forwarding headers cannot satisfy either socket check.

The library exposes exactly one MCP embedding entry point:
`McpRouterBuilder::try_new`. It requires an already-bound listener, a validated
authentication policy, validated request limits, an exact transport-security
policy, and safe-root inputs that it validates and lifetime-pins. The builder
always installs authentication outermost, followed by authenticated
`Content-Length` rejection, concurrency admission and timeout, the streaming
body limit and extraction, Host/Origin validation, and only then HTTP/MCP
lifecycle, discovery, grant, tool, and mutation handling. Raw transport state,
raw router construction, legacy constructor variants, transport options, and
capability-authority bundles are crate-private, test-only, or absent. The
authentication policy is an opaque public type whose bearer principal cannot
be destructured by an embedding; its `Debug` output remains redacted. The
package binary uses this same builder. See [`EMBEDDING.md`](EMBEDDING.md).

The optional runtime is not a broad host-control surface. After authentication, it enforces bounded concurrency, request duration, and request-body size, then validates exact `Host` and browser `Origin` allowlists before dispatch. It exposes only the currently documented staged tools:

- `runtime_status`
- `platform_info`
- `android_status`
- `project_service_status`
- `create_directory`
- `copy_file`
- `trash_file`
- `find_paths`
- `hash_file`
- `list_directory`
- `path_metadata`
- `read_binary_file`
- `read_binary_range`
- `read_file`
- `read_text_range`
- `search_text`
- `write_file`

An `android-battery-status` build may additionally expose `android_battery_status` only when `MCP__ANDROID__BATTERY_STATUS_ENABLED=true`. The runtime flag defaults to disabled and is rejected if the compile feature is absent. The provider directly executes one fixed Termux:API program with no arguments, null stdin, a cleared inherited environment, a five-second normal-operation budget with a reserved cleanup window, and hard stdout/stderr ceilings. A single cancellation-safe supervisor isolates the provider process group, terminates it immediately on overflow or cancellation, closes both pipes, and synchronously reaps the direct child. If reaping misses the reserve, the stable wait-failure result becomes authoritative and the supervisor remains responsible until collection. It returns only normalized allowlisted fields and never reflects technology/vendor strings, identifiers, raw output, stderr, paths, or environment values.

An `android-volume-status` build may additionally expose `android_volume_status` only when `MCP__ANDROID__VOLUME_STATUS_ENABLED=true`. Its runtime flag has the same compile/runtime fail-closed relationship. It directly executes only the fixed `termux-volume` path with zero arguments, so callers cannot reach the upstream command's mutation mode. The shared Android provider supervisor applies the same environment, process-group, cancellation, cleanup, and reaping guarantees with 8 KiB/4 KiB output ceilings. Parsing requires the exact six official streams and exact integer fields; it canonicalizes output order and rejects unknown, duplicate, missing, extra, or range-invalid data without reflection.

An `android-volume-control` build may expose `set_android_volume` only when `MCP__ANDROID__VOLUME_CONTROL_ENABLED=true`, static-token authentication, and a complete capability key pair are active. Preview is the default and reads fresh strict status without invoking the setter. Live mutation requires one 60-second grant bound to the keyed principal, canonical session, capability, exact stream, exact level, and mutating posture. The grant is consumed immediately before fixed two-argument execution and remains consumed. One non-queueing permit rejects conflicts; fresh status verifies success; setter or verification failure triggers restoration to the captured prior level. An owned worker continues verification/recovery after caller cancellation and owns the exactly-once terminal audit outcome independently of the HTTP waiter. See [`ANDROID_VOLUME_CONTROL.md`](ANDROID_VOLUME_CONTROL.md).

A `command-execution` build may additionally expose `run_command_profile` only when `MCP__COMMAND__ENABLED=true` and initialization succeeds. Command enablement is structurally confined to the binary target: `src/main.rs` compiles the module graph in the binary crate and alone can call the crate-private command switch on `McpRouterBuilder`; the single public builder defaults the lane off and exposes no command-enablement method. No mintable command-authority token exists; ordinary dependency and selected-workspace compile probes prove the binary-only switch, raw execution types, and all legacy router construction surfaces remain unreachable. Initialization opens the exact-name absolute `current_exe` candidate without following its final component, independently opens `/proc/self/exe`, and requires an executable regular candidate plus a regular loaded image with identical device/inode identity; profiles then launch only `/proc/self/exe`. The first canonical safe root is retained as a no-follow directory descriptor, filesystem-root aliases are rejected by device/inode, and the child uses `/proc/self/fd/<fd>` while a guard remains alive through execution. Project-owned argv, an empty environment, null stdin, immutable 5-second/16 KiB stdout/4 KiB stderr maxima, two non-queueing permits, process-group cleanup, and authoritative reaping remain enforced. The request and public Rust API cannot select a program, forge or inspect a resolved profile, reach raw argv, or override cwd, environment, stdin, timeout, or limits. Failures suppress child output and use stable reasons.

Android controls beyond the request-authorized exact-stream volume capability, shell fallback, arbitrary command execution, global process inventory, arbitrary service inspection, service mutation/control, package management, network mutation, and unrelated high-impact controls remain disabled.

## Authentication and Startup Behavior

Startup requires `MCP__AUTH__STATIC_TOKEN` by default. Empty or whitespace-only values are rejected before the HTTP listener starts. The configured token is redacted from debug output and must not be logged or copied into issue reports.

Bearer authentication does not by itself authorize `create_directory`, `copy_file`, `trash_file`, or `write_file` mutation. Each has an independent default-disabled gate and requires a paired 32-byte HMAC key configuration plus one 60-second single-use `MCP-Capability-Grant`. Directory grants bind an absent target; copy grants bind exact source identity/content and absent destination; trash grants bind one exact single-link target identity/content plus fixed recovery retention; write grants bind exact content, disposition, and replacement identity. Each payload is only 65 opaque bytes (130 lowercase hex): random JTI, distinct signed family byte, keyed operation binding, and issued/expiry timestamps—never raw binding material. The runtime consumes the JTI immediately before the authorized namespace/publication mutation and retains consumption after downstream failure. Grants are capability-distinct, header-only, and must never appear in arguments, URLs, responses, logs, audit labels, tickets, or screenshots. See the four focused `*_CAPABILITY_GRANTS.md` documents, including [`TRASH_FILE_CAPABILITY_GRANTS.md`](TRASH_FILE_CAPABILITY_GRANTS.md).

Bearer authentication likewise does not authorize Android volume mutation by itself. The volume-control runtime gate and the same private key configuration are required, but its signed capability code and exact stream/level binding are distinct from directory grants. The public Rust client exposes preview only; preparation, the prepared mutation value, and execution are crate-private so an embedding cannot bypass request-grant validation. See [`ANDROID_VOLUME_CONTROL.md`](ANDROID_VOLUME_CONTROL.md).

Only an absent environment variable may select its documented default. Present non-Unicode values for the bearer token, listener host/port, safe roots, transport allowlists, compatibility switch, or request limits fail startup with non-sensitive errors. `MCP__SERVER__PORT` accepts only `1–65535`; port `0` is not a supported supervised-listener configuration.

The only supported exception is explicit local development mode:

```bash
export MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true
export MCP__SERVER__HOST=127.0.0.1
```

This opt-in is rejected for non-loopback bind addresses and every request is rechecked against its actual socket peer. Do not use unauthenticated mode with tunnels, LAN exposure, reverse proxies, shared devices, or any remotely reachable deployment.

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
| `POST`, `GET`, `DELETE /mcp` stable transport | Disabled | Bearer-authenticated and resource-bounded except explicit loopback development mode; JSON/GET-405 default with bounded SSE opt-in |
| `runtime_status` / `platform_info` | Disabled | Read-only |
| `android_status` | Disabled | Read-only allowlisted metadata |
| `android_battery_status` | Disabled | Available only in the `android-battery-status` build with explicit runtime opt-in; bounded read-only telemetry |
| `android_volume_status` | Disabled | Available only in the `android-volume-status` build with explicit runtime opt-in; bounded read-only telemetry |
| `set_android_volume` | Disabled | Available only in the `android-volume-control` build with explicit runtime opt-in; preview by default, live mutation requires one exact single-use grant and verified recovery semantics |
| `run_command_profile` | Disabled | Available only through the package binary's crate-private builders in the `command-execution` build with explicit runtime opt-in and successful executable initialization; three fixed read-only diagnostics |
| `project_service_status` | Disabled | Read-only allowlisted project service metadata |
| `create_directory` | Disabled | Preview is available; mutation is separately default-disabled and requires fixed mode `0700`, atomic no-replace, and one request-scoped single-use grant |
| `copy_file` | Disabled | One regular file up to 1 MiB, fixed mode `0600`, atomic no-replace, content-private, dry-run by default |
| `trash_file` | Disabled | Preview is available; live mode moves one exact single-link file into a private bounded recovery quarantine after an identity/content-bound grant |
| `find_paths` | Disabled | Literal basename discovery with no-follow traversal, kind/depth filters, 8,192-entry and 512-match ceilings, content-free bounded response |
| `hash_file` | Disabled | One no-follow regular file up to 16 MiB, streaming SHA-256, digest-and-size-only response |
| `list_directory` | Disabled | Bounded and safe-rooted |
| `path_metadata` | Disabled | Bounded, content-free, descriptor-relative metadata |
| `read_binary_file` | Disabled | One no-follow regular file up to 1 MiB, canonical padded base64, fixed complete-response ceiling, no path or host metadata |
| `read_binary_range` | Disabled | One range up to 256 KiB from a no-follow regular file up to 64 MiB, canonical padded base64, explicit EOF, fixed complete-response ceiling, no path or host metadata |
| `read_file` | Disabled | Bounded UTF-8 and safe-rooted |
| `read_text_range` | Disabled | One code-point-safe UTF-8 range up to 256 KiB from a no-follow regular file up to 64 MiB, explicit continuation/EOF metadata, fixed worst-case escaped-response ceiling, no path or host metadata |
| `search_text` | Disabled | Bounded literal locations without content excerpts |
| `write_file` | Disabled | Preview remains available; live mutation is independently default-disabled and requires an exact content/disposition/identity-bound single-use grant, mode-`0600` publication, and bounded displaced-object retention for replacement |
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

SSE is separately default-disabled. When enabled, each eligible request gets an unguessable stream ID, an empty primer, and one terminal JSON-RPC event. `Last-Event-ID` is accepted only after normal authentication and session validation, is limited to one canonical 64-byte server-issued value, and can access only later events from its exact originating stream. Eight streams, two events per stream, 128 KiB per event, and 256 KiB total replay per session are hard ceilings. Every HTTP 200 JSON-RPC family is serialized under one aggregate response ceiling before SSE selection; canonical serialized request IDs are capped at 1 MiB before dispatch and before initialization allocates a session. Eligible tool results remain in process memory until oldest-first eviction, DELETE, idle expiry, or restart; operators handling sensitive safe-root output should retain the JSON default unless resumability is necessary. Cross-session, cross-stream, unknown, and evicted cursors share one non-reflective 404.

Session identifiers are UUID v4 values, but they are not authentication credentials. The server re-runs bearer authentication for every request before session lookup. The in-memory store retains no client-provided identity or capability metadata, holds at most 64 sessions, expires idle records after 30 minutes, and supports explicit DELETE termination. Protect session identifiers from disclosure anyway: a caller possessing both the shared operator bearer token and an active session ID can act within that session. All `/mcp` handler responses use `Cache-Control: no-store`.

## Request Resource Limits

The MCP transport uses explicit limits intended for a supervised mobile process:

- `MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS`: default `4`, valid `1–64`.
- `MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS`: default `30`, valid `1–300`.
- `MCP__TRANSPORT__MAX_BODY_BYTES`: default `2097152`, valid `1024–8388608`.
- `MCP__TRANSPORT__SSE_ENABLED`: default `false`, strict boolean opt-in.

Values outside these ranges fail startup validation. Concurrency saturation fails fast with HTTP 503 and `Retry-After: 1`. Request timeout returns HTTP 504. Request bodies over the configured ceiling return HTTP 413. All limit responses use non-sensitive JSON and `Cache-Control: no-store`.

The body ceiling is implemented with Axum's streaming extractor limit rather than a second full body buffer. This keeps peak memory usage predictable on Termux. The request timeout covers body extraction and dispatch; an authorized write continues in a cancellation-independent worker. Replacement recovery objects are deliberately retained in a bounded private quarantine rather than automatically removed after a timed-out waiter or post-commit failure.

## Filesystem and Tool Safety Rules

Filesystem requests must lexically identify a descendant of a configured safe root. Fallible startup accepts one through 64 configured entries and rejects empty or relative entries, filesystem root `/`, parent traversal, missing or non-directory objects, and every symlink in a root or its ancestors after the exact listener is bound but before its router is built or any request is served. Valid root labels are normalized, sorted, and deduplicated; they remain request-matching metadata rather than filesystem authority.

Startup opens each root component-by-component with directory/path-descriptor and no-follow semantics, records the final device/inode identity, and retains that descriptor for the runtime lifetime. Every tools clone shares the pinned set. Each live create, copy, trash, find, hash, list, metadata, read, search, and write duplicates the selected pin, verifies its identity, and resolves descendants one component at a time with descriptor-relative `openat` plus `O_NOFOLLOW`; it never reopens the configured pathname as authority. Directory enumeration, source reads, hashing, staging, retention, publication, cleanup, and metadata lookup remain relative to held descriptors. Reserved write/trash quarantine names and directory identities are hidden across every filesystem surface. Path discovery classifies children without following links, traverses only verified directories, skips invalid-UTF-8 names and special objects, and never reads file content. Copy and trash additionally verify exact source/target identity and bytes around bounded reads. Hashing and both binary-read tools verify that the opened regular descriptor matches the pre-open no-follow device/inode observation. Range reads also retain the initial descriptor size and reject a size change detected after the bounded seek/read.

Renaming or replacing a configured root or any ancestor cannot redirect a running runtime: operations continue against the original pinned directory and leave a replacement at the configured path untouched. Restart is the explicit repinning boundary. Offline create/copy/trash/write grant issuers use the same fallible pinning contract; runtime preparation and grant consumption compare the grant's root device/inode binding with the running pin. Issuance against a later pathname replacement therefore fails to authorize the earlier running root. Deterministic adversarial tests cover root and ancestor replacement, clone sharing, and grant-binding mismatch. Errors, debug output, audit labels, and production evidence expose neither configured root paths nor retained descriptor/device/inode metadata.

The default safe root is deliberately narrow:

```text
/data/data/com.termux/files/home/mcp-files
```

Broad shared-storage roots such as `/storage/emulated/0` and `/sdcard` are not defaults. Safe-root entries are not trimmed: whitespace is path data and a value that becomes relative because of leading whitespace fails closed. Operators must stop the service before replacing or remounting a configured storage hierarchy, then restart and verify readiness so the intended objects are freshly validated and pinned.

`create_directory`, `copy_file`, `trash_file`, `find_paths`, `hash_file`, `list_directory`, `path_metadata`, `read_binary_file`, `read_binary_range`, `read_file`, `read_text_range`, `search_text`, and `write_file` are response or payload bounded. Directory, copy, trash, and write default to preview. Each live mutation requires its independent gate and exact request grant; `dry_run:false` alone is never authorization. One process-global create/copy/trash/write publication lock covers posture revalidation, cancellation ownership, grant consumption, mutation, verification, and durability. Trash accepts one exact single-link target up to 1 MiB, revalidates identity and SHA-256, then moves the inode with `NOREPLACE` into its separate private recovery quarantine only after consumption. Both quarantine namespaces are inaccessible through every MCP filesystem operation and capped at 32 regular artifacts and 32 MiB per parent. Copy, trash, and write preflight their complete 16 KiB responses before path access and return no private path, content, digest, or internal name.

Path discovery accepts one literal basename query of at most 256 bytes, examines at most 8,192 entries to depth 5, returns at most 512 ordered file/directory matches, and preflights its 262,144-byte response before argument parsing or filesystem access. File hashing streams arbitrary bytes from one regular descriptor up to 16 MiB, returns only lowercase SHA-256 and bytes hashed, preflights its 16 KiB response before reading, and never records digest/path/content in audits. Whole-file binary read accepts arbitrary bytes from one regular descriptor up to 1 MiB and preflights its 1,507,328-byte response. Binary range read accepts at most 256 KiB from one regular file up to 64 MiB and preflights its 393,216-byte response. Both binary tools return canonical padded base64 and fixed size/limit metadata. Text range read accepts 4 to 256 KiB from one UTF-8 regular file up to 64 MiB, defers an incomplete trailing code point, returns only complete UTF-8 plus byte-continuation metadata, and preflights its 1,703,936-byte worst-case escaped response. Range tools reject offset-past-EOF and detected size changes and never record path/content/file identity in audits. Directory listings are deterministic and response bounded. Metadata and path discovery are content-free; whole-file text reads accept at most 1 MiB of valid UTF-8; text search returns only bounded locations. The replacement exchange is the commit point: a later failure can leave the authorized new target and retained prior object, and the grant remains consumed. No automatic rollback or destructive post-capture cleanup is claimed against a hostile same-UID namespace race.

Quarantine locks are advisory and nonblocking. They coordinate cooperating runtime writers but cannot constrain a process running under the same Unix UID; such a process can cause contention, mutate names, or exhaust storage elsewhere. The 32-entry/32-MiB ceilings are per target parent, not global disk bounds. Operators must stop the service and other same-UID writers before inspecting or manually maintaining selected recovery artifacts. Live filesystem mutation must use safe roots under the service's exclusive operational ownership; independent writers must remain inactive while any create, copy, trash, or write gate is enabled.

Read-only metadata tools must not expose environment values, raw secrets, persistent device identifiers, global process inventories, unrelated service state, or command output.

The battery and volume providers are not general command runners. Callers cannot choose their executable, arguments, stdin, environment, timeout, output limit, or parsed fields. Disabled, unavailable, timeout, overflow, process, and parsing failures return stable reason codes without process paths, exit details, or raw output. Neither tool grants Android device-control or audio-mutation authority.

## Audit Counter Privacy

The staged runtime exposes in-memory aggregate audit counters through `runtime_status`. Counters retain stable tool names, allowed/denied totals, and low-cardinality reason codes only.

They must not retain raw paths, file contents, command arguments or output, environment names or values, bearer tokens, capability grants or keys, principal fingerprints, sessions, JTIs, target digests, timestamps, hostnames, usernames, Android identifiers, or arbitrary caller strings.

Audit counters provide evidence of gate decisions; they are not authorization and reset when the process restarts. Authentication failures are deliberately handled before MCP tool audit counters because unauthorized callers must not enter the MCP dispatch path.

## Command and High-Impact Capability Boundaries

The fixed-profile command gate is live only in its separate build, after runtime opt-in, and through the package binary's crate-private builder switch. The public builder is structurally command-disabled. It authorizes three read-only diagnostics of the inode-pinned running image, not a shell or general process launcher. Its complete boundary is documented in [`command-execution-gate.md`](command-execution-gate.md).

The narrowly scoped `create_directory`, `copy_file`, `trash_file`, `write_file`, and Android-volume request-grant primitives are live only for their exact bound operations. Public library entry points remain preview-only; crate-private preparation/execution is reachable only by the grant-aware transport. They share private key configuration and one public header name, while the registry assigns unique signed family codes: directory `1`, write `2`, volume `3`, copy `4`, and trash `5`. Codes and target encodings are not caller-selectable, and exhaustive cross-family tests prove rejection outside each family without consumption. The separate general-purpose capability-token policy module remains inert scaffolding.

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
