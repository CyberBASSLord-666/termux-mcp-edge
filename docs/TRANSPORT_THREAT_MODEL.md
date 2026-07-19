# MCP Transport Threat Model

This document defines the security boundary for the stable MCP 2025-11-25 Streamable HTTP transport compiled by the optional `mcp-runtime` feature and its deliberately staged tool authority.

## Current Runtime Boundary

The default build exposes unauthenticated coarse `GET /health` and `GET /ready` probes. It does not compile the MCP route or MCP tools.

The `mcp-runtime` build additionally exposes authenticated `POST`, `GET`, and `DELETE /mcp` handling with:

- bearer authentication, except for explicit loopback-only development mode whose declared bind and actual TCP peer must both be loopback;
- bounded concurrency, request duration, and body size;
- exact Host and browser Origin validation;
- strict single-message JSON-RPC request, notification, and response classification;
- stable protocol negotiation, per-session initialization gating, and exact subsequent-request protocol headers;
- at most 64 cryptographically random UUID sessions with a 30-minute idle expiry and explicit DELETE termination;
- the sixteen-tool baseline allowlist, including bounded content-free path discovery and code-point-safe UTF-8 range pagination, plus only those battery, volume-status, exact-grant volume-control, and fixed-command tools whose independent gates are active, as documented in README and the authorization policy;
- safe-root, payload, dry-run, request-capability, and audit-counter controls for the current filesystem surface.

POST requires JSON content and explicit client support for JSON and SSE responses. Accepted notifications and client responses return HTTP 202 without a body. The default transport returns JSON and the specification-permitted HTTP 405 for GET. `MCP__TRANSPORT__SSE_ENABLED=true` permits finite two-event SSE request responses and cursor-bearing GET resumption. Each response stream receives an unguessable UUID-derived identity, an empty priming event, and one terminal JSON-RPC event. Replay is held inside the originating session only, never broadcast, and bounded to 8 streams, 2 events per stream, 128 KiB per event, 256 KiB per session, and a 64-byte `Last-Event-ID`. Every HTTP 200 response is preflighted under the aggregate collector ceiling, and canonical serialized JSON-RPC request IDs are capped at 1 MiB before dispatch or session allocation. Oldest streams are evicted deterministically; oversized eligible responses remain JSON. DELETE and idle expiry remove both lifecycle and replay state.

## Assets to Protect

- The configured bearer token and active session identifiers.
- Termux home data and configured filesystem safe roots.
- Android shared storage and app-private data outside those roots.
- SSH keys, API keys, cookies, tunnel credentials, and other device-local secrets.
- Local-network resources reachable from the Android device.
- Process, package, service, shell, Shizuku/rish, and Android-control boundaries.
- MCP client identity, lifecycle state, request integrity, and tool authorization decisions.
- The shared configured HMAC key plus the independently capability-bound directory, file-write, and volume grants, consumed-JTI state, and target/content/disposition/identity binding integrity.
- Mobile memory, CPU, battery, thermal, and process-lifetime budgets.

## Threats and Current Controls

### Unauthenticated discovery or invocation

An untrusted local, LAN, tunnel, or browser-originated caller may attempt to enumerate or invoke tools.

Current controls:

- static bearer authentication wraps the complete MCP route before resource accounting, transport validation, parsing, discovery, and dispatch;
- credential parsing and comparison are bounded;
- failures return one non-sensitive 401 contract with `WWW-Authenticate: Bearer` and `Cache-Control: no-store`;
- unauthenticated mode requires explicit configuration and a loopback bind;
- request-time `ConnectInfo<SocketAddr>` must prove the actual TCP peer is IPv4 or IPv6 loopback. Missing metadata and non-loopback peers receive the same private denial before request resource accounting; `Host`, `Origin`, and forwarded headers are never substitutes for the socket peer.

Preserved boundary:

- lifecycle and session identifiers never replace request authentication;
- authentication failures must not allocate MCP sessions, consume tool concurrency, or enter tool audit counters.

### Browser rebinding and ambient browser access

A malicious page can target loopback or LAN listeners through DNS rebinding, redirects, or ambient browser authority.

Current controls:

- exact normalized Host allowlists;
- exact HTTP/HTTPS Origin allowlists;
- rejection of malformed authorities, wildcard/userinfo/path/query/fragment forms, whitespace/control characters, malformed IP literals, and invalid ports;
- missing Origin is denied unless an operator explicitly enables the reviewed non-browser compatibility switch.

Preserved boundary:

- Streamable HTTP GET, POST, DELETE, and unsupported-method handling share the same Origin and authentication policy;
- reverse proxies or tunnels must not broaden allowlists implicitly.

### Cross-client lifecycle or session confusion

One client may attempt to reuse another client's state, request identifiers, session identifier, resumability cursor, or initialization status.

Current controls:

- valid initialize params are required before a session is allocated, and the server always negotiates its supported stable version `2025-11-25`;
- each session has independent pending/active lifecycle state, and only ping is accepted as a request before `notifications/initialized`;
- every session-bearing request is independently authenticated before lookup, so a session ID never grants authority by itself;
- UUID v4 identifiers are generated locally, contain visible ASCII only, and are never derived from or paired with retained client metadata;
- the single static token represents one operator security principal; possession of both that credential and a session identifier is treated as the same principal rather than a distinct end-user identity;
- missing protocol/session headers fail with HTTP 400, unsupported protocol versions fail with HTTP 400, and unknown, expired, or terminated sessions fail with HTTP 404 without reflecting presented identifiers;
- the store holds at most 64 sessions, expires them after 30 minutes of inactivity, prunes during session operations, and is cleared on process shutdown;
- DELETE provides explicit shutdown; after deletion, expiry, or process restart, clients reconnect with a new initialize request;
- SSE is default-disabled; when enabled, only server-issued canonical cursors are accepted, exact event lookup occurs after authentication and session validation, replay never crosses the originating stream, oversized request IDs are rejected before initialization can orphan capacity, and unknown, evicted, or cross-session cursors share one non-reflective 404 contract;
- each finite SSE stream contains only its primer and terminal response, so concurrent streams cannot broadcast or consume another stream's events; syntactically valid client responses remain HTTP 202 and create no replay state;
- cancellation notifications are accepted without a JSON-RPC response. A live `write_file` operation moves into one cancellation-independent blocking transaction before consuming its grant, so a dropped or timed-out request does not abandon the transaction. Grant consumption and the irreversible exchange remain authoritative even when the HTTP waiter disappears; there is no separate long-lived operation registry to cancel across HTTP requests.

### Resource exhaustion

Authenticated callers may use concurrent requests, large bodies, slow handlers, large filesystem results, or repeated errors to exhaust a mobile process.

Current controls:

- fail-fast concurrency admission;
- total request timeout;
- streaming request-body ceiling and early oversized `Content-Length` rejection;
- bounded directory-creation responses, file reads, write payloads, directory depth, and entry count;
- a separate two-permit non-queueing semaphore plus fixed deadlines and stream ceilings for command diagnostics.
- SSE replay has fixed event, stream, cursor, and retained-byte ceilings; count or byte pressure evicts the oldest complete stream, and responses above the event ceiling use the existing bounded JSON path. The collector uses the largest registered complete-response contract, while every successful response family preflights that aggregate ceiling.
- enabling SSE intentionally retains eligible serialized tool responses in process memory until eviction, DELETE, idle expiry, or restart; the default remains JSON-only so operators must opt into that confidentiality/liveness tradeoff.

Deterministic filesystem response-byte budgets and single-content serialization landed through #206. One-directory creation has a 16 KiB full-response ceiling. One-file copy has a 1 MiB source ceiling, a pre-mutation 16 KiB full-response ceiling, a fixed-size content-free result, and no subprocess or caller-selected resource controls. `write_file` accepts at most 1 MiB of UTF-8 and preflights its complete 16 KiB content/path-free response, including the actual caller-controlled JSON-RPC ID, before filesystem access or grant consumption. Single-object metadata has a 16 KiB full-response ceiling and never reads content or returns host identifiers. Binary read has a 1 MiB raw ceiling, an exact 1,398,104-byte canonical base64 maximum, and a pre-access 1,507,328-byte complete-response ceiling that includes the actual JSON-RPC ID. Literal search adds fixed query, traversal, byte, match, and response ceilings and returns no content. Any future long-lived streaming expansion must independently add connection, queue, heartbeat, shutdown, and backpressure bounds rather than reusing the finite replay posture implicitly.

### Filesystem escape and mutation

An authenticated caller may attempt traversal, symlink escape, race-based path replacement, oversized output, or unintended mutation.

Current controls:

- absolute dedicated safe roots with no filesystem-root default;
- rejection of explicit parent traversal, NUL bytes, unsafe missing parents, and symlink components;
- safe-root descriptor anchoring and component-by-component no-follow descendant resolution;
- bounded deterministic UTF-8 and canonical base64 reads plus directory traversal;
- dry-run-by-default directory/file mutation and explicit `dry_run:false`;
- a separately default-disabled directory-mutation gate plus one 60-second, single-use HMAC grant bound to the static principal, canonical session, exact root identity, normalized target, and mutating posture;
- an independently default-disabled `MCP__FILE__WRITE_MUTATION_ENABLED` gate plus one 60-second, single-use HMAC grant bound to the static principal, canonical session, exact root identity, normalized target, exact UTF-8 content digest, `create`/`replace` disposition, mutating posture, and exact existing regular-file device/inode for replacement;
- confinement and response preflight before grant matching, atomic JTI consumption immediately before the first mutation attempt, concurrent replay exclusion, and retained consumption after downstream failure;
- one-directory creation with existing parents, fixed mode `0700`, unpredictable staging, atomic no-replace publication, descriptor sync, and identity-checked cleanup;
- one-file write with a fixed per-parent mode-`0700` `.termux-mcp-write-quarantine`, one randomized mode-`0600` staging inode, file sync and exact identity/size/mode verification, atomic `NOREPLACE` for create, or one identity-verified irreversible `EXCHANGE` for replace;
- replacement eligibility restricted to a single-link regular target of at most 1 MiB; the exchange leaves the displaced prior inode/content under the randomized quarantine name and never automatically unlinks, truncates, chmods, or swaps it back after capture;
- quarantine isolation from all MCP filesystem operations, at most 32 regular artifacts and 32 MiB per target parent, and a nonblocking advisory lock for cooperating writers;
- one cancellation-independent live-write transaction after authorization, a fixed 1 MiB content ceiling, and a content/path-free 16 KiB complete response.

The exchange is the replacement commit point. If a later identity or durability check fails, the authorized new inode may remain at the target while the displaced object remains quarantined; the request is denied and the grant stays consumed. The preservation policy avoids destructive cleanup against an uncertain name, but it is not atomic rollback against a malicious process under the same Unix UID. Such a peer can ignore the advisory lock, mutate names, or exhaust per-parent capacity, so the limits bound cooperating runtime retention rather than global disk use. Production deployment therefore requires exclusive operational ownership of mutation safe roots by this service, with no independent writers active while live create or write gates are enabled.

The focused remediation and regression evidence landed through #200, #206, #240, #242, #244, #247, #248, #261, #262, and #203. Any future filesystem expansion must preserve these descriptor, response, authorization, and deployment boundaries.

### Request-grant theft, replay, and confused deputy use

An authenticated caller may try to reuse a grant for another session, root, target, content, create/replace disposition, replacement identity, posture, request method, tool, or time window; race the same JTI concurrently; smuggle it through arguments; or provoke a later failure and retry.

Current controls:

- grants are issued only by the local exact binary after it independently anchors the configured safe root and validates the capability-specific target. The file-write issuer additionally reads exact UTF-8 bytes from an absolute stable no-follow private regular file of at most 1 MiB and validates the requested `create`/`replace` target state;
- create-directory grants use their own fixed-shape signed binding. A write grant instead has an opaque 65-byte payload (random JTI, signed write-family byte, keyed domain-separated operation binding, issued/expiry timestamps) plus its outer signature: principal, canonical session, capability, root/target, posture, content digest, disposition, and replacement device/inode/size/high-resolution ctime/link count are binding inputs and are not serialized;
- a single internal registry assigns stable globally unique family codes—directory `1`, write `2`, volume `3`, and reserved copy `4`. Callers cannot select a code, and exhaustive all-pairs tests with one key/principal/session prove every live grant is privately rejected by both other families without consuming the source grant;
- route authentication is outermost. Host/Origin, HTTP method, media/header shape, JSON-RPC envelope, session/lifecycle, `tools/call`, exact grant-aware tool name, closed argument schema, runtime gate, complete-response preflight, safe-root/target classification, and grant binding are checked in that order before the first state change;
- only one bounded ASCII `MCP-Capability-Grant` header is accepted. It is rejected on initialization, GET, DELETE, ping, discovery, notifications, client responses, and unrelated tools; an active-session `tools/call` still authorizes nothing unless its exact capability authority validates it;
- malformed, unknown-key/version, invalid-signature, expired, future, excessive-lifetime, mismatched, replayed, clock-rollback, full-state, and poisoned-state cases fail closed with non-sensitive stable reasons;
- a mutex makes validation plus replay insertion atomic, so concurrent replay reaches at most one mutation attempt;
- live directory and file-write requests share one fixed service-owned, fail-fast, non-queueing mutation-worker permit. Exhaustion returns private HTTP 503 / JSON-RPC `-32007` before preparation, grant consumption, or mutation. The permit lives through descriptor preparation and the complete blocking commit even after the HTTP timeout releases its request-concurrency permit;
- after descriptor preparation, one poison-fail-closed process-wide publication lock serializes every `create_directory` and `write_file` instance and transport state through posture revalidation, grant consumption, mutation, verification, and durability. A stale prepared loser fails before grant consumption. While holding that lock, a reusable atomic commit guard races request cancellation against worker ownership immediately before grant consumption. A cancellation winner, including one that arrived while waiting for the process lock, consumes no grant and mutates nothing. A worker winner consumes the directory or write grant immediately before its first mutation and owns completion; consumption then survives staging, sync, exchange, recovery, publication, response, timeout, and disconnect outcomes;
- dry-run and rejected-context requests cannot consume the grant;
- responses, tracing, audit labels, CLI errors, and production evidence never serialize the header, key, principal fingerprint, session, JTI, target/content digest, disposition-bound filesystem identity, private path/content, artifact name/count/bytes, or bound time.

Residual boundary:

- a process restart clears the bounded in-memory replay set. Grants expire after 60 seconds; rotate the key on restart when immediate invalidation is required.
- a caller that steals the bearer token, active session ID, exact operation knowledge, and an unexpired grant can attempt that one exactly bound mutation. Protect all of them and keep the listener/transport allowlists narrow.

### Schema confusion and response reflection

Callers may send extra fields, wrong JSON types, oversized field names, or malformed tool arguments that differ from advertised schemas.

Current controls:

- strict top-level JSON-RPC envelope classification;
- a closed `tools/call` parameter envelope and closed typed argument structs for every advertised tool;
- one shared validator that accepts only omitted arguments or `{}` for no-argument tools;
- stable bounded invalid-parameter responses that do not expose serde diagnostics or rejected caller values;
- request-body limits and non-sensitive audit labels.

### Fixed command diagnostics and high-impact capability exposure

Command execution, Android control, service/package/network mutation, broad storage, process inspection, and credential handling can turn an MCP server into a full device-compromise path.

Current controls:

- the only live process-execution surface is a separately compiled and runtime-enabled `run_command_profile` tool for three read-only diagnostics; it additionally requires an opaque authority obtainable by the Cargo primary package but not by dependency builds, and all public embedding routers are command-disabled;
- initialization requires an executable regular `current_exe` with exact basename `termux-mcp-server`; later execution uses `/proc/self/exe` so installation-path replacement cannot redirect a profile to another inode;
- its closed schema accepts no program, argv, path, environment, stdin, timeout, or limit input, while crate-private profiles, resolved handles, and raw execution types prevent a downstream Rust embedding from forging or inspecting those values;
- empty environment, null stdin, safe-root cwd, bounded streams/deadline/concurrency, process-group cleanup, zero-exit/UTF-8 success, and non-sensitive audit reasons are enforced; independent 5-second/16-KiB/4-KiB supervisor maxima reject oversize configuration before spawn, and capacity grows fallibly only from bytes actually read;
- arbitrary commands, shells, broader Android/service/package/network mutation, broad inspection, credentials, and unrelated high-impact capabilities are absent from discovery and dispatch;
- the narrow `create_directory`, `write_file`, and exact-stream volume request-grant modules are live only for their distinct bound mutations; the separate general capability-token policy module remains inert;
- public directory-creation, file-write, and Android-volume library APIs expose preview only. Their live preparation and execution are crate-private, preventing an embedding from routing around those transport authorities. Command execution is absent from public embedding routers, and the dependency-mode authority probe prevents safe acquisition of the primary-server handle. The legacy public `copy_file` live path is a separately tracked trusted-embedding limitation until the reserved copy-grant family is implemented;
- read-only Android and service metadata use fixed allowlists and expose no control path.

Required controls for expansion:

- each future capability family requires a separate compile/runtime gate, fixed allowlist, bounded input/output, explicit operator consent, audit coverage, and focused validation;
- no arbitrary shell string or silent fallback to broader authority is permitted.

### Dependency and protocol drift

Rust dependencies, GitHub Actions, the Android toolchain, and the MCP specification change independently.

Current controls:

- immutable action pins, pinned Rust 1.88.0, pinned Android NDK, exact-head CI, RustSec Security, and six-posture Android validation;
- dependency changes remain separate from unrelated runtime behavior.

Required controls:

- keep the adopted stable MCP revision explicit and treat any later revision or SSE expansion as a focused compatibility change;
- require Security and dependency-alert review for graph changes;
- remove unused dependency features instead of retaining hypothetical surface.

## Merge Blockers

A transport or tool-surface change is blocked when it:

- weakens authentication, Host/Origin enforcement, resource limits, or safe-root policy;
- permits unauthenticated discovery/invocation in static-token mode;
- introduces lifecycle/session state without authenticated isolation and bounded cleanup;
- claims stable MCP conformance without the required protocol behavior and tests;
- exposes a high-impact capability without an independent gate;
- reflects secrets, raw paths, raw I/O text, or unbounded caller-controlled values;
- lacks exact-head CI or required Android/Security evidence;
- combines unrelated dependency, protocol, deployment, and capability changes in one review unit.

## Required Validation Evidence

Every affected change must identify the exact head SHA and provide the applicable evidence:

1. host format, Clippy, all-feature tests, and deployment shell tests;
2. all six Android AArch64 builds and applicable native ARM64 official-Termux gates for device-affecting source/toolchain changes;
3. RustSec Security and dependency-alert review for dependency changes;
4. accepted and rejected authentication, Host, Origin, envelope, lifecycle, and tool-schema cases;
5. concurrency, timeout, body, response, cancellation, cleanup, and reconnect bounds appropriate to the change;
6. proof that disabled high-impact tools remain absent from discovery and invocation;
7. operator documentation that describes only implemented behavior.

For a file-write change, evidence additionally includes the independent disabled/enabled discovery truth table, exact-binary issuer, all grant bindings and replay failures, exact-limit/content-free response checks including `recoveryArtifactRetained`, create `NOREPLACE` without retention, irreversible replace `EXCHANGE` with preserved recovery material, quarantine namespace/capacity/lock failures, post-commit fault states, and deterministic release-validator plus native Termux device-smoke coverage against the exact artifact.

