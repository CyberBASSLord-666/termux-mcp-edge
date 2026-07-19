# Termux MCP Edge (Rust)

Termux MCP Edge is a hardened Rust/Axum HTTP service for Android Termux deployments. The default runtime exposes health and readiness endpoints and enforces fail-closed authentication posture at startup.

The project is designed for developers, advanced Termux operators, and power users who understand that MCP tools can affect local device state. Capabilities are introduced through explicit opt-in configuration, allowlists, bounded inputs and outputs, dry-run or preview behavior, tests, and audit coverage.

The optional `mcp-runtime` feature wires a stable MCP 2025-11-25 Streamable HTTP `/mcp` transport around the staged tool surface. In static-token mode, bearer authentication is enforced before resource-limit accounting, transport validation, JSON-RPC parsing, lifecycle handling, tool discovery, or tool invocation. Authenticated requests must pass mobile-conscious concurrency, timeout, body-size, exact `Host`, and browser `Origin` checks.

The transport negotiates protocol version `2025-11-25`, issues bounded cryptographically random sessions, requires `notifications/initialized` before normal operations, enforces media and protocol headers, accepts one JSON-RPC request, notification, or response per POST, and supports explicit session termination. JSON responses remain the default. An independent default-disabled runtime option adds finite SSE request responses and exact originating-stream replay without creating an unbounded connection or queue subsystem.

## Current runtime scope

- **Runtime:** Rust single binary using Axum.
- **Source package version:** `0.6.0` release candidate. No `v0.6.0` tag or GitHub Release is authoritative until the final exact-main release procedure completes.
- **Operational endpoints:** `GET /health` and `GET /ready`.
- **Optional MCP endpoint:** authenticated Streamable HTTP `POST`, `GET`, and `DELETE /mcp` handling when built with `--features mcp-runtime`; GET returns 405 by default, while the explicit SSE posture accepts only cursor-bearing replay GETs.
- **Staged MCP discovery:** `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `create_directory`, `copy_file`, `find_paths`, `hash_file`, `list_directory`, `path_metadata`, `read_binary_file`, `read_binary_range`, `read_file`, `read_text_range`, `search_text`, and `write_file`; independent battery, volume-status, fixed-command, and request-authorized volume-control builds may additionally expose their narrowly bounded tool after explicit runtime opt-in.
- **Filesystem surface:** deterministic bounded directory listing, content-free literal basename discovery, single-object metadata, streaming SHA-256 hashing, canonical base64 whole-file and range reads, UTF-8 reads, literal text search, one-directory creation, bounded binary file copy, and file writes. Mutations are descriptor-relative, crash-durable, dry-run by default, independently default-disabled, and each requires its own 60-second request-scoped single-use grant. A copy grant binds the authenticated principal, active session, both anchored roots and normalized paths, exact single-link source identity/size/high-resolution ctime, SHA-256 of the exact bytes, absent destination, and no-replace posture. A write grant additionally binds exact UTF-8 content, create-or-replace disposition, and—when replacing—the exact existing file identity. Live copy and writes cap content at 1 MiB and publish mode `0600`; copy and write-create use atomic no-replace, while write-replace uses one irreversible exchange and retains the displaced prior inode/content in a private bounded per-parent recovery quarantine. Copy staging is hidden in that mode-`0700` quarantine and successful results return neither endpoint path nor content. Path discovery examines at most 8,192 entries through no-follow directory descriptors and returns at most 512 literal basename matches under a 262,144-byte response ceiling. Whole-file binary read accepts one no-follow regular file up to 1 MiB. Binary range read accepts a 256 KiB slice from one no-follow regular file up to 64 MiB. Both return canonical padded RFC 4648 base64 without path or host metadata. Hashing accepts one no-follow regular file up to 16 MiB and returns only its lowercase SHA-256 digest and byte count. Metadata, hashing, path discovery, and text search remain content-private under fixed response and traversal ceilings.
- **Authentication:** startup fails closed unless a non-empty static token is configured or explicit localhost-only development mode is enabled.
- **Transport ordering:** authentication precedes MCP resource limits, exact Host/Origin validation, body parsing, and dispatch.
- **Mobile defaults:** four concurrent authenticated MCP requests, a 30-second request timeout, and a 2 MiB request body.
- **Session bounds:** 64 in-memory UUID sessions with a 30-minute idle expiry; client initialization metadata is validated but not retained. Opt-in SSE state is owned by the session and bounded to 8 streams, 2 events per stream, 128 KiB per event, 256 KiB retained per session, and 64-byte cursors.
- **Default filesystem root:** `/data/data/com.termux/files/home/mcp-files`.
- **Project service name:** `mcp_runtime`.
- **Deployment:** versioned Termux releases with atomic activation, health/readiness validation, and rollback.
- **Named tunnels:** explicit, non-overwriting Cloudflare Tunnel setup with strict hostname validation and hermetic failure-path tests.

Android controls other than the separately compiled, request-authorized exact-stream volume tool remain unavailable, as do shell fallback, arbitrary command execution, global process inspection, arbitrary service control, package management, and network mutation. The optional battery and volume-status tools are bounded read-only telemetry. The optional command posture runs only three fixed diagnostics of the exact server binary and does not authorize a shell or caller-selected command surface.

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

## Directory mutation authorization

`create_directory` preview remains available in the baseline tool registry, but mutation is disabled by default. Explicit `dry_run:false` is necessary and not sufficient. Production mutation requires:

```dotenv
MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

It also requires static-token authentication and one locally issued, target-bound grant in the `MCP-Capability-Grant` request header. The exact server binary issues a grant with `--issue-create-directory-grant` after the caller supplies the active canonical session ID and absent target through `MCP__CAPABILITY__SESSION_ID` and `MCP__CAPABILITY__CREATE_DIRECTORY_TARGET`; `MCP__CAPABILITY__CONFIG_FILE` lets the offline issuer read the exact private deployed `runtime.env` through a bounded no-follow literal parser without shell evaluation. Grants are never tool arguments. After descriptor preparation, the runtime acquires the process-global create/copy/write publication lock and revalidates target absence before resolving the request-cancellation/worker commit guard: cancellation or stale-target failure consumes no grant and mutates nothing, while a worker that wins owns completion, consumes the grant immediately before the first mutation, and preserves consumption after every later outcome. See [`docs/CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](docs/CREATE_DIRECTORY_CAPABILITY_GRANTS.md) for secure configuration, issuance, use, rotation, denial reasons, and validation order.

## File-copy mutation authorization

`copy_file` is also preview-only unless its independent gate is enabled. Create, write, and Android-volume gates or grants cannot authorize it. Production copy requires:

```dotenv
MCP__FILE__COPY_FILE_MUTATION_ENABLED=true
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

The exact deployed binary issues a grant with `--issue-copy-file-grant`. The operator supplies the active session and private absolute source/destination inputs through `MCP__CAPABILITY__SESSION_ID`, `MCP__CAPABILITY__COPY_FILE_SOURCE`, and `MCP__CAPABILITY__COPY_FILE_DESTINATION`. The issuer independently opens and hashes the exact single-link source; caller-supplied identity, size, or digest is never trusted. The opaque grant binds both root identities and normalized paths, source identity/size/high-resolution ctime/SHA-256, absent destination, and no-replace posture. Under the shared create/copy/write publication lock, the runtime revalidates the exact source bytes and destination absence before cancellation ownership and grant consumption, then stages mode `0600` inside the MCP-hidden mode-`0700` quarantine and atomically publishes with `NOREPLACE`. Results contain no endpoint path, content, digest, grant, or staging name. See [`docs/COPY_FILE_CAPABILITY_GRANTS.md`](docs/COPY_FILE_CAPABILITY_GRANTS.md).

## File-write mutation authorization

`write_file` likewise remains available for bounded preview while live mutation is independently disabled. Enabling directory mutation does not enable file writing, and a directory grant cannot authorize it. Production writes require:

```dotenv
MCP__FILE__WRITE_MUTATION_ENABLED=true
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

The exact deployed binary issues a grant with `--issue-write-file-grant`. The operator supplies the active session, exact target, a private no-follow mode-`0600` content file, and an explicit `create` or `replace` disposition through `MCP__CAPABILITY__SESSION_ID`, `MCP__CAPABILITY__WRITE_FILE_TARGET`, `MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE`, and `MCP__CAPABILITY__WRITE_FILE_DISPOSITION`. The grant conceptually binds the exact content digest and, for replacement, the exact preflight device, inode, size, high-resolution ctime, and one-link identity. Its payload is nevertheless opaque: 65 bytes (130 lowercase hex) containing only a random ID, the signed write-family byte, keyed operation binding, and issued/expiry timestamps—not raw request or filesystem binding data. After descriptor preparation, the process-global publication lock covers target-posture revalidation, cancellation/worker ownership, grant consumption, and private publication. A stale target or cancellation winner consumes no grant and changes no target; a worker winner publishes the new file at mode `0600` and preserves consumption after every later outcome. Replacement retains the displaced object under a randomized name in a reserved mode-`0700` quarantine; create retains none. See [`docs/WRITE_FILE_CAPABILITY_GRANTS.md`](docs/WRITE_FILE_CAPABILITY_GRANTS.md) for issuance, transaction, recovery-artifact maintenance, rotation, privacy, and adversarial validation requirements.

Live `create_directory`, `copy_file`, and `write_file` requests share one fixed service-owned mutation-worker permit. Admission never queues: exhaustion returns private HTTP 503 / JSON-RPC `-32007` before descriptor preparation, grant consumption, or mutation. The permit remains owned throughout blocking preparation and commit even if the request timeout releases its ordinary HTTP-concurrency permit. A separate process-global publication lock serializes the pre-consumption posture check through verified publication across every embedded router. Equivalent authority instances also share replay, last-observed-clock, and capacity state inside one process; multi-process consumers require an external atomic one-use coordinator.

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

Send `notifications/initialized` before discovery or invocation. Accepted notifications and client responses return HTTP 202 with no body. With the default `MCP__TRANSPORT__SSE_ENABLED=false`, POST responses remain JSON and valid GET returns HTTP 405. Setting it to `true` allows bounded JSON-RPC responses up to 128 KiB to use a finite two-event SSE stream: an empty priming event with a one-second reconnect hint followed by the terminal JSON-RPC response. Larger responses stay JSON rather than entering replay memory. If a POST stream disconnects after an event, reconnect with GET plus its exact `Last-Event-ID`; the server replays only later events from that session and originating stream. Missing cursors still receive 405, malformed cursors receive 400, and unavailable, evicted, or cross-session cursors receive the same non-reflective 404. DELETE terminates the session and its replay state with HTTP 204; expired, terminated, or unknown session IDs return HTTP 404. Session IDs and cursors scope lifecycle state but never replace bearer authentication.

Local unauthenticated development requires both:

```bash
export MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true
export MCP__SERVER__HOST=localhost
```

This mode is rejected for non-loopback binds. At request time the server also requires connection metadata proving the actual TCP peer is IPv4 or IPv6 loopback; missing metadata and non-loopback peers fail closed before request limits or MCP parsing. This check uses the socket peer, not `Host`, `Origin`, or forwarded headers. Do not combine the mode with tunnels, LAN exposure, or reverse proxies.

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

`MCP__TRANSPORT__SSE_ENABLED=true` is an operator opt-in for finite response delivery and bounded resumption. Eligible response payloads—including tool output—remain in process memory until stream eviction, session deletion, idle expiry, or restart. It does not enable server-initiated broadcast, long-lived queues, or cross-stream delivery. `/ready` and `runtime_status` report the active posture and fixed limits.

## MCP request resource limits

| Setting | Default | Valid range |
|---|---:|---:|
| `MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS` | `4` | `1–64` |
| `MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS` | `30` | `1–300` |
| `MCP__TRANSPORT__MAX_BODY_BYTES` | `2097152` | `1024–8388608` |

Unsafe values fail startup validation. Saturation returns HTTP 503 with `Retry-After: 1`; timeout returns HTTP 504; oversized bodies return HTTP 413. Limit failures use non-sensitive JSON and `Cache-Control: no-store`.

Authentication is the outer gate, so unauthenticated traffic does not consume MCP concurrency permits or body-buffer capacity. `/ready` reports the active non-sensitive limit values when `mcp-runtime` is enabled.

## Filesystem safe roots

The service does not default to broad Android shared storage. Keep `MCP__FILE__SAFE_ROOTS` limited to dedicated project directories. Empty root lists or entries, relative roots, filesystem root `/`, traversal, and symlink components are rejected. Live create/copy/find/hash/list/metadata/binary-read/text-read/search/write operations walk from opened safe-root descriptors with no-follow semantics for every descendant instead of authorizing one pathname and using it later. [`docs/SAFE_ROOT_DIRECTORY_CREATION.md`](docs/SAFE_ROOT_DIRECTORY_CREATION.md) defines directory creation; [`docs/CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](docs/CREATE_DIRECTORY_CAPABILITY_GRANTS.md) defines its separate authorization layer; [`docs/COPY_FILE_CAPABILITY_GRANTS.md`](docs/COPY_FILE_CAPABILITY_GRANTS.md) defines exact source/destination copy authorization; [`docs/WRITE_FILE_CAPABILITY_GRANTS.md`](docs/WRITE_FILE_CAPABILITY_GRANTS.md) defines the separately gated content- and identity-bound write authority; [`docs/SAFE_ROOT_FILE_COPY.md`](docs/SAFE_ROOT_FILE_COPY.md) defines bounded content-private file copy; [`docs/SAFE_ROOT_PATH_DISCOVERY.md`](docs/SAFE_ROOT_PATH_DISCOVERY.md) defines literal basename discovery; [`docs/SAFE_ROOT_FILE_HASHING.md`](docs/SAFE_ROOT_FILE_HASHING.md) defines bounded descriptor-relative hashing; [`docs/SAFE_ROOT_BINARY_READS.md`](docs/SAFE_ROOT_BINARY_READS.md) and [`docs/SAFE_ROOT_BINARY_RANGES.md`](docs/SAFE_ROOT_BINARY_RANGES.md) define canonical binary reads; [`docs/SAFE_ROOT_TEXT_RANGES.md`](docs/SAFE_ROOT_TEXT_RANGES.md) defines code-point-safe UTF-8 range pagination; [`docs/SAFE_ROOT_PATH_METADATA.md`](docs/SAFE_ROOT_PATH_METADATA.md) defines metadata; [`docs/SAFE_ROOT_TEXT_SEARCH.md`](docs/SAFE_ROOT_TEXT_SEARCH.md) defines literal-search limits.

## Optional Android battery telemetry

Battery telemetry requires a separately compiled and separately enabled posture:

```bash
cargo build --release --features android-battery-status
export MCP__ANDROID__BATTERY_STATUS_ENABLED=true
```

The feature includes `mcp-runtime`. Startup rejects the runtime flag when the feature is absent. With the flag unset or `false`, `android_battery_status` is hidden from discovery. With both gates enabled, the server directly invokes only the fixed Termux:API `termux-battery-status` executable with no arguments, no stdin, no inherited environment, a five-second normal-operation budget with a reserved cleanup window, and independent 16 KiB/4 KiB stdout/stderr limits. A cancellation-safe supervisor isolates and terminates the complete provider process group on overflow, timeout, request cancellation, or completion and reaps the direct child without unbounded reader joins. If reaping exhausts the reserved window, a stable wait failure overrides the primary result and the supervisor remains responsible until the child is collected. Only normalized allowlisted battery fields are returned; technology/vendor strings, identifiers, raw output, stderr, paths, and environment values are discarded.

See [`docs/ANDROID_BATTERY_STATUS.md`](docs/ANDROID_BATTERY_STATUS.md) for prerequisites, field units, failure reason codes, audit behavior, and validation evidence.

## Optional Android volume telemetry

Audio-stream volume status requires an independent compile-time and runtime opt-in:

```bash
cargo build --release --features android-volume-status
export MCP__ANDROID__VOLUME_STATUS_ENABLED=true
```

The feature includes `mcp-runtime`. Startup rejects the runtime flag when the feature is absent, and the tool remains hidden when the flag is unset or `false`. With both gates enabled, `android_volume_status` executes only the fixed Termux:API `termux-volume` executable in its zero-argument read-only mode. The shared cancellation-safe Android provider supervisor fixes the working directory, clears the environment, supplies null stdin, enforces a five-second budget plus 8 KiB/4 KiB output ceilings, isolates and cleans the complete process group, and authoritatively reaps the direct child. The parser requires the exact six official audio streams and exact fields, rejects vendor extensions or malformed/range-invalid values, and returns a canonical bounded response. Callers cannot reach the upstream command's argument-taking mutation mode.

See [`docs/ANDROID_VOLUME_STATUS.md`](docs/ANDROID_VOLUME_STATUS.md) for the exact stream contract, authority boundary, stable failures, audit behavior, and native ARM64 evidence.

## Optional request-authorized Android volume control

One exact audio-stream mutation requires its own compile and runtime gates:

```bash
cargo build --release --features android-volume-control
export MCP__ANDROID__VOLUME_CONTROL_ENABLED=true
```

The gate additionally requires static-token authentication plus the paired capability key used for request grants. `set_android_volume` defaults to preview, accepts only the six documented streams and one integer level, validates the fresh live maximum, and never exposes a shell or caller-selected execution setting. Explicit `dry_run:false` requires a 60-second principal/session/stream/level-bound grant issued locally by the exact binary with `--issue-android-volume-grant`. The server consumes the grant immediately before the fixed two-argument setter, verifies fresh status afterward, and attempts confirmed restoration to the captured prior level on setter or verification failure. Conflicting mutations fail without queueing, and recovery continues if the request is cancelled after grant consumption. The detached task—not the HTTP waiter—records exactly one terminal verified/recovery audit outcome.

See [`docs/ANDROID_VOLUME_CONTROL.md`](docs/ANDROID_VOLUME_CONTROL.md) for configuration, issuance, invocation, recovery, stable outcomes, audit privacy, and release evidence.

## Optional fixed-profile command diagnostics

Fixed server diagnostics require a separate build and runtime opt-in:

```bash
cargo build --release --features command-execution
export MCP__COMMAND__ENABLED=true
```

The feature includes `mcp-runtime`. A default build rejects the runtime flag, while a command build with the flag unset hides `run_command_profile` and denies direct calls without spawning. Enabled callers may choose only `server_version`, `server_help`, or `execution_boundary`. Every profile runs the exact current executable with fixed argv, the first canonical safe root as cwd, empty environment, null stdin, a five-second deadline, independent output ceilings, a two-permit non-queueing concurrency limit, process-group cleanup, zero-exit enforcement, and UTF-8-only bounded output. Callers cannot supply a command, program, argv, path, environment, stdin, timeout, or limit.

See [`docs/command-execution-gate.md`](docs/command-execution-gate.md) for the complete request, response, failure, audit, and native validation contract.

## Build and validate

Run the exact CI gates:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
```

Build all supported postures:

```bash
cargo build --release
cargo build --release --features mcp-runtime
cargo build --release --features android-battery-status
cargo build --release --features android-volume-status
cargo build --release --features android-volume-control
cargo build --release --features command-execution
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

Before a release declaration, run the no-clone exact-commit AArch64 device gate in [`docs/DEVICE_PRODUCTION_GATE.md`](docs/DEVICE_PRODUCTION_GATE.md). The harness builds the pinned source natively in Termux and validates real isolated runit transitions, authenticated MCP lifecycle and tool boundaries, request-limit ordering, failed upgrade/rollback recovery, explicit rollback, uninstall, artifact identity, and cleanup.

Validate the exact downloaded default, `mcp-runtime`, and `android-volume-control` Android candidates together through [`docs/RELEASE_CANDIDATE_VALIDATION.md`](docs/RELEASE_CANDIDATE_VALIDATION.md). Each workflow bundle includes an exact-source manifest and checksum sidecar; validator v8 reconciles all three with the supplied commit/run metadata, executes the sixteen-tool baseline including content-free path discovery, bounded file hashing, canonical binary reads, code-point-safe UTF-8 range pagination, and request-granted file-write mutation, proves the control posture remains hidden and inert by default without changing device audio, requires explicit confirmation for runtime/deployment phases, and emits only versioned sanitized JSON evidence.

The Android workflow additionally executes the default, `mcp-runtime`, opt-in battery, read-only volume, request-authorized volume-control, and fixed-command postures in the digest-pinned official Termux container on a native ARM64 runner. Feature gates also consume an incompatible artifact where needed to prove compile-time rejection. [`docs/EMULATED_RELEASE_GATE.md`](docs/EMULATED_RELEASE_GATE.md) defines the automated gates, the evidence-only classification used when runtime changes require later physical release evidence, and the narrow conditions under which a completed physical observation may be inherited by a metadata-only descendant.

Use [`docs/operator-validation.md`](docs/operator-validation.md) for authenticated MCP, audit-counter, filesystem, Android-status, service-status, and capability-boundary checks.

## Project documentation

- [Operations guide](docs/OPERATIONS.md)
- [Security guide](docs/SECURITY.md)
- [Validation guide](docs/VALIDATION.md)
- [Safe-root binary range contract](docs/SAFE_ROOT_BINARY_RANGES.md)
- [Safe-root UTF-8 text range contract](docs/SAFE_ROOT_TEXT_RANGES.md)
- [Safe-root path discovery contract](docs/SAFE_ROOT_PATH_DISCOVERY.md)
- [Production readiness checklist](docs/PRODUCTION_READINESS.md)
- [Transport threat model](docs/TRANSPORT_THREAT_MODEL.md)
- [MCP runtime validation plan](docs/MCP_RESTORATION_VALIDATION.md)
- [MCP runtime roadmap](docs/MCP_RUNTIME_ROADMAP.md)
- [Safe-rooted directory creation contract](docs/SAFE_ROOT_DIRECTORY_CREATION.md)
- [`create_directory` request-capability grants](docs/CREATE_DIRECTORY_CAPABILITY_GRANTS.md)
- [`copy_file` request-capability grants](docs/COPY_FILE_CAPABILITY_GRANTS.md)
- [`write_file` request-capability grants](docs/WRITE_FILE_CAPABILITY_GRANTS.md)
- [Safe-rooted file copy contract](docs/SAFE_ROOT_FILE_COPY.md)
- [Safe-rooted file hashing contract](docs/SAFE_ROOT_FILE_HASHING.md)
- [Safe-rooted path metadata contract](docs/SAFE_ROOT_PATH_METADATA.md)
- [Safe-rooted text-search contract](docs/SAFE_ROOT_TEXT_SEARCH.md)
- [Safe-rooted path-discovery contract](docs/SAFE_ROOT_PATH_DISCOVERY.md)
- [Android artifact contract](docs/ANDROID_ARTIFACTS.md)
- [Android battery status tool](docs/ANDROID_BATTERY_STATUS.md)
- [Android volume status tool](docs/ANDROID_VOLUME_STATUS.md)
- [Request-authorized Android volume control](docs/ANDROID_VOLUME_CONTROL.md)
- [Fixed-profile command diagnostics](docs/command-execution-gate.md)
- [Command profile validation runbook](docs/command-profile-validation.md)
- [Exact-commit Termux device production gate](docs/DEVICE_PRODUCTION_GATE.md)
- [Downloaded release-candidate validation](docs/RELEASE_CANDIDATE_VALIDATION.md)
- [Native ARM64 Termux emulated release gate](docs/EMULATED_RELEASE_GATE.md)
- [v0.6.0 release-candidate record](docs/V0.6.0_RELEASE_CANDIDATE.md)
- [Termux deployment and recovery](docs/TERMUX_DEPLOYMENT.md)
- [Operator validation checklist](docs/operator-validation.md)

## Architecture

- Rust 2021 single binary.
- Axum HTTP runtime.
- Minimal internal stable MCP 2025-11-25 Streamable HTTP transport with a default-disabled bounded SSE/replay posture and no external MCP framework dependency.
- `termux-services` / runit supervision.
- Localhost-first networking with explicit authenticated remote-access posture.
- Staged capability gates for higher-risk developer and power-user functionality.
