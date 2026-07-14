# MCP Transport Threat Model

This document defines the security boundary for the stable MCP 2025-11-25 Streamable HTTP transport compiled by the optional `mcp-runtime` feature and its deliberately staged tool authority.

## Current Runtime Boundary

The default build exposes unauthenticated coarse `GET /health` and `GET /ready` probes. It does not compile the MCP route or MCP tools.

The `mcp-runtime` build additionally exposes authenticated `POST`, `GET`, and `DELETE /mcp` handling with:

- bearer authentication, except for explicit loopback-only development mode;
- bounded concurrency, request duration, and body size;
- exact Host and browser Origin validation;
- strict single-message JSON-RPC request, notification, and response classification;
- stable protocol negotiation, per-session initialization gating, and exact subsequent-request protocol headers;
- at most 64 cryptographically random UUID sessions with a 30-minute idle expiry and explicit DELETE termination;
- the nine-tool baseline allowlist, plus only those read-only battery, volume, and fixed-command tools whose independent compile/runtime gates are both enabled, as documented in README and the authorization policy;
- safe-root, payload, dry-run, and audit-counter controls for the current filesystem surface.

POST requires JSON content and explicit client support for JSON and SSE responses. Accepted notifications and client responses return HTTP 202 without a body. GET validates the same authentication, Host, Origin, protocol, and session boundaries, then returns the specification-permitted HTTP 405 because the server does not initiate SSE streams. Consequently there is no replay buffer, event cursor, or resumability state. DELETE removes a valid session and returns HTTP 204.

## Assets to Protect

- The configured bearer token and active session identifiers.
- Termux home data and configured filesystem safe roots.
- Android shared storage and app-private data outside those roots.
- SSH keys, API keys, cookies, tunnel credentials, and other device-local secrets.
- Local-network resources reachable from the Android device.
- Process, package, service, shell, Shizuku/rish, and Android-control boundaries.
- MCP client identity, lifecycle state, request integrity, and tool authorization decisions.
- Mobile memory, CPU, battery, thermal, and process-lifetime budgets.

## Threats and Current Controls

### Unauthenticated discovery or invocation

An untrusted local, LAN, tunnel, or browser-originated caller may attempt to enumerate or invoke tools.

Current controls:

- static bearer authentication wraps the complete MCP route before resource accounting, transport validation, parsing, discovery, and dispatch;
- credential parsing and comparison are bounded;
- failures return one non-sensitive 401 contract with `WWW-Authenticate: Bearer` and `Cache-Control: no-store`;
- unauthenticated mode requires explicit configuration and a loopback bind.

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
- server-initiated requests, SSE, replay, and resumption are not implemented, so there are no cross-stream request IDs or cursors to confuse; syntactically valid client responses are accepted and discarded with HTTP 202;
- cancellation notifications are accepted without a JSON-RPC response, while request timeout and cancellation-safe write cleanup bound work; there is no separate long-lived operation registry to cancel across HTTP requests.

### Resource exhaustion

Authenticated callers may use concurrent requests, large bodies, slow handlers, large filesystem results, or repeated errors to exhaust a mobile process.

Current controls:

- fail-fast concurrency admission;
- total request timeout;
- streaming request-body ceiling and early oversized `Content-Length` rejection;
- bounded file reads, write payloads, directory depth, and entry count;
- a separate two-permit non-queueing semaphore plus fixed deadlines and stream ceilings for command diagnostics.

Deterministic filesystem response-byte budgets and single-content serialization landed through #206. Single-object metadata has a 16 KiB full-response ceiling and never reads content or returns inode/device/UID/GID/mode/access-time values. Literal search adds fixed query, entry, file, per-file, aggregate-byte, match, and response ceilings; returns no content; and performs no regex or subprocess evaluation. Any future SSE/replay implementation must independently bound connections, queues, event IDs, replay buffers, and reconnect behavior before exposure.

### Filesystem escape and mutation

An authenticated caller may attempt traversal, symlink escape, race-based path replacement, oversized output, or unintended mutation.

Current controls:

- absolute dedicated safe roots with no filesystem-root default;
- rejection of explicit parent traversal, NUL bytes, unsafe missing parents, and symlink components;
- safe-root descriptor anchoring and component-by-component no-follow descendant resolution;
- bounded deterministic UTF-8 reads and directory traversal;
- dry-run-by-default writes, explicit `dry_run:false`, payload limits, descriptor-relative mode-0600 temporary files, file sync, atomic rename, and parent-directory sync.

The focused remediation and regression evidence landed through #200, #206, and #203 respectively. Any future filesystem expansion must preserve these descriptor, response, and deployment boundaries.

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

- the only live process-execution surface is a separately compiled and runtime-enabled `run_command_profile` tool for three read-only diagnostics of the exact server executable;
- its closed schema accepts no program, argv, path, environment, stdin, timeout, or limit input;
- empty environment, null stdin, safe-root cwd, bounded streams/deadline/concurrency, process-group cleanup, zero-exit/UTF-8 success, and non-sensitive audit reasons are enforced;
- arbitrary commands, shells, Android/service/package/network mutation, broad inspection, credentials, and other high-impact capabilities are absent from discovery and dispatch;
- capability-token modules remain inert policy scaffolding;
- read-only Android and service metadata use fixed allowlists and expose no control path.

Required controls for expansion:

- each future capability family requires a separate compile/runtime gate, fixed allowlist, bounded input/output, explicit operator consent, audit coverage, and focused validation;
- no arbitrary shell string or silent fallback to broader authority is permitted.

### Dependency and protocol drift

Rust dependencies, GitHub Actions, the Android toolchain, and the MCP specification change independently.

Current controls:

- immutable action pins, pinned Rust 1.88.0, pinned Android NDK, exact-head CI, RustSec Security, and five-posture Android validation;
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
2. all five Android AArch64 builds and applicable native ARM64 official-Termux gates for device-affecting source/toolchain changes;
3. RustSec Security and dependency-alert review for dependency changes;
4. accepted and rejected authentication, Host, Origin, envelope, lifecycle, and tool-schema cases;
5. concurrency, timeout, body, response, cancellation, cleanup, and reconnect bounds appropriate to the change;
6. proof that disabled high-impact tools remain absent from discovery and invocation;
7. operator documentation that describes only implemented behavior.
