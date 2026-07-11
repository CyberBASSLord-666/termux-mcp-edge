# MCP Transport Threat Model

This document defines the security boundary for the staged MCP transport currently compiled by the optional `mcp-runtime` feature and the controls required for its migration to stable MCP 2025-11-25 Streamable HTTP.

## Current Runtime Boundary

The default build exposes unauthenticated coarse `GET /health` and `GET /ready` probes. It does not compile the MCP route or MCP tools.

The `mcp-runtime` build additionally exposes authenticated `POST /mcp` with:

- bearer authentication, except for explicit loopback-only development mode;
- bounded concurrency, request duration, and body size;
- exact Host and browser Origin validation;
- strict JSON-RPC request/notification envelope classification;
- the seven-tool allowlist documented in README and the authorization policy;
- safe-root, payload, dry-run, and audit-counter controls for the current filesystem surface.

The transport is still a custom POST-only stage. It reports protocol version `2024-11-05` but does not implement complete initialization-state enforcement, stable 2025-11-25 Streamable HTTP GET/POST behavior, media negotiation, the `MCP-Protocol-Version` header contract, or optional session/resumption behavior. It must not be represented as fully MCP-conformant until #199 lands.

## Assets to Protect

- The configured bearer token and any future session identifiers.
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

Required preservation:

- future lifecycle or session identifiers must never replace request authentication;
- authentication failures must not allocate MCP sessions, consume tool concurrency, or enter tool audit counters.

### Browser rebinding and ambient browser access

A malicious page can target loopback or LAN listeners through DNS rebinding, redirects, or ambient browser authority.

Current controls:

- exact normalized Host allowlists;
- exact HTTP/HTTPS Origin allowlists;
- rejection of malformed authorities, wildcard/userinfo/path/query/fragment forms, whitespace/control characters, malformed IP literals, and invalid ports;
- missing Origin is denied unless an operator explicitly enables the reviewed non-browser compatibility switch.

Required preservation:

- Streamable HTTP GET and POST must share the same Origin and authentication policy;
- reverse proxies or tunnels must not broaden allowlists implicitly.

### Cross-client lifecycle or session confusion

One client may attempt to reuse another client's state, request identifiers, session identifier, resumability cursor, or initialization status.

Current limitation:

- the staged transport has no complete per-client MCP lifecycle/session implementation.

Required controls for #199:

- negotiate a supported protocol version during `initialize`;
- require `notifications/initialized` before normal operations;
- bind any session state to the authenticated client context;
- generate non-predictable session identifiers if sessions are enabled;
- validate `MCP-Protocol-Version` on subsequent requests;
- define cancellation, shutdown, reconnection, and resumption behavior without cross-client state leakage.

### Resource exhaustion

Authenticated callers may use concurrent requests, large bodies, slow handlers, large filesystem results, or repeated errors to exhaust a mobile process.

Current controls:

- fail-fast concurrency admission;
- total request timeout;
- streaming request-body ceiling and early oversized `Content-Length` rejection;
- bounded file reads, write payloads, directory depth, and entry count.

Remaining work:

- #206 must add deterministic filesystem response-byte budgets and avoid duplicate large content in MCP results;
- protocol work must bound SSE connections, queues, replay buffers, session state, and reconnect behavior.

### Filesystem escape and mutation

An authenticated caller may attempt traversal, symlink escape, race-based path replacement, oversized output, or unintended mutation.

Current controls:

- absolute dedicated safe roots with no filesystem-root default;
- rejection of explicit parent traversal, NUL bytes, unsafe missing parents, and static symlink escapes;
- bounded UTF-8 reads and directory traversal;
- dry-run-by-default writes, explicit `dry_run:false`, payload limits, same-directory temporary files, and atomic rename.

Remaining work:

- #200 must replace canonicalize-then-use pathname operations with descriptor-relative no-follow/beneath access to close symlink/directory race windows;
- #206 must bound and determinize serialized results;
- #203 must complete atomic deployment/service transition behavior around the runtime binary.

### Schema confusion and response reflection

Callers may send extra fields, wrong JSON types, oversized field names, or malformed tool arguments that differ from advertised schemas.

Current controls:

- strict top-level JSON-RPC envelope classification;
- a closed `tools/call` parameter envelope and closed typed argument structs for every advertised tool;
- one shared validator that accepts only omitted arguments or `{}` for no-argument tools;
- stable bounded invalid-parameter responses that do not expose serde diagnostics or rejected caller values;
- request-body limits and non-sensitive audit labels.

### High-impact capability exposure

Command execution, Android control, service/package/network mutation, broad storage, process inspection, and credential handling can turn an MCP server into a full device-compromise path.

Current controls:

- none of those live capabilities is compiled into discovery or dispatch;
- command and capability-token modules are inert policy scaffolding;
- read-only Android and service metadata use fixed allowlists and expose no control path.

Required controls:

- each future capability family requires a separate compile/runtime gate, fixed allowlist, bounded input/output, explicit operator consent, audit coverage, and focused validation;
- no arbitrary shell string or silent fallback to broader authority is permitted.

### Dependency and protocol drift

Rust dependencies, GitHub Actions, the Android toolchain, and the MCP specification change independently.

Current controls:

- immutable action pins, pinned Rust 1.88.0, pinned Android NDK, exact-head CI, RustSec Security, and two-posture Android validation;
- dependency changes remain separate from unrelated runtime behavior.

Required controls:

- target the latest adopted stable MCP specification explicitly rather than a draft or superseded transport;
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
2. default and `mcp-runtime` Android AArch64 builds for device-affecting source/toolchain changes;
3. RustSec Security and dependency-alert review for dependency changes;
4. accepted and rejected authentication, Host, Origin, envelope, lifecycle, and tool-schema cases;
5. concurrency, timeout, body, response, cancellation, cleanup, and reconnect bounds appropriate to the change;
6. proof that disabled high-impact tools remain absent from discovery and invocation;
7. operator documentation that describes only implemented behavior.
