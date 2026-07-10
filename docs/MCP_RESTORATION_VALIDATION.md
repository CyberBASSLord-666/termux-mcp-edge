# MCP Restoration Validation Plan

This document defines validation evidence for the current staged MCP transport and for future protocol and capability expansion.

The default build remains a conservative Axum health/readiness service. The optional `mcp-runtime` build exposes authenticated staged MCP discovery and the documented allowlisted tool set. The current POST endpoint is a staged custom transport; it must preserve JSON-RPC message rules and may not be described as a complete standard HTTP-with-SSE/session implementation until those stages land.

## Required PR shape

MCP work must remain staged through small, reviewable pull requests. Do not combine broad dependency restoration, protocol/lifecycle changes, high-impact tool exposure, and unrelated maintenance.

For future expansion:

1. Define the protocol or capability contract and threat model.
2. Add independently testable validation/policy primitives.
3. Add or update authentication, authorization, resource, and audit boundaries.
4. Add disabled-by-default runtime wiring.
5. Add allowed, denied, boundary, notification, recovery, and regression tests.
6. Add operator documentation and rollback/recovery notes.

## Exact-head gate

Every MCP runtime PR must prove:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo build --release
cargo build --release --features mcp-runtime
```

CI must succeed on the exact head. Security must succeed when Cargo, lockfile, or Security-workflow inputs change. Android cross-compilation is required when runtime, dependency, workflow, or deployment changes can affect the device artifact.

## Dependency gate

Before merge, dependency review must confirm one of:

- no dependency changes;
- exact-head Security and dependency alerts are clear;
- any accepted advisory has documented scope, impact, mitigation, owner, and review date.

Unused dependencies must be removed rather than retained for hypothetical future work.

## Authentication, resource, and transport security tests

| Scenario | Expected result |
| --- | --- |
| Missing authorization in static-token mode | HTTP 401 before MCP resource accounting or message handling. |
| Malformed, oversized, or incorrect bearer token | HTTP 401 with no sensitive detail. |
| Valid bearer token | Request reaches request limits and Host/Origin validation. |
| Concurrency saturated | HTTP 503 with bounded non-sensitive response. |
| Request duration exceeded | HTTP 504. |
| Request body exceeded | HTTP 413. |
| Unexpected `Host` | HTTP 403 before JSON-RPC dispatch. |
| Unexpected browser `Origin` | HTTP 403 before JSON-RPC dispatch. |
| Loopback-only unauthenticated mode on loopback | Development-only access is allowed. |
| Loopback-only unauthenticated mode on non-loopback bind | Startup fails closed. |
| Debug/error output | Tokens, private paths, and caller payloads remain absent. |

`/health` and `/ready` may remain unauthenticated only while responses remain coarse and non-sensitive.

## JSON-RPC and MCP message-envelope tests

Every transport implementation must distinguish malformed JSON from a valid JSON value that is not a valid request object.

Required evidence:

- malformed JSON returns `-32700 Parse error` with `id: null`;
- valid non-object JSON, unsupported batch arrays in the current stage, missing/wrong `jsonrpc`, missing/non-string method, invalid ID types, and primitive params return `-32600 Invalid Request`;
- `jsonrpc` is exactly `"2.0"`;
- MCP request IDs are non-null strings or integer numbers;
- params are object/array when present;
- notifications omit ID and receive no JSON-RPC response;
- `notifications/initialized` receives HTTP 204 with an empty body;
- notification-shaped request methods are not dispatched and cannot mutate state;
- unsupported notifications receive no response;
- valid request IDs are preserved in request errors, while invalid IDs are never reflected.

The current focused envelope stage does not claim batch, session, lifecycle, or SSE completion. Those require a separate design and integration stage.

## Lifecycle and standard transport completion gate

Before claiming complete MCP 2024-11-05 interoperability, the runtime must implement and test:

- initialization as the first client/server interaction;
- protocol-version and capability negotiation;
- receipt of `notifications/initialized` before normal operation;
- per-client/session state and request-ID uniqueness where applicable;
- the standard HTTP-with-SSE connection model or a fully documented custom transport that preserves lifecycle and bidirectional requirements;
- cancellation, shutdown, reconnect, and multiple-client behavior;
- batch behavior if supported, or a documented compatibility decision supported by the selected MCP schema/transport.

## Tool discovery tests

Any discovery change must prove:

1. unauthenticated callers receive no tools;
2. authenticated callers receive only the intended staged set;
3. high-impact tools remain absent until explicitly gated;
4. schemas and descriptions match runtime behavior;
5. discovery does not expose secrets, private paths, process inventory, or credential-bearing arguments.

## Tool invocation tests

Any invocation change must prove:

1. unauthenticated invocation is rejected;
2. unknown or unauthorized tools are rejected;
3. notification-shaped tool calls do not dispatch;
4. disabled high-impact tools cannot be invoked;
5. enabled tools enforce safe roots, bounds, dry-run/mutation rules, and audit privacy;
6. failures do not leak secrets, private paths, raw I/O errors, command arguments, tokens, or personal data.

## High-impact tool requirements

High-impact tools include command execution, package management, process/service control, Android/device control, broad filesystem mutation, network mutation, browser automation, credential handling, and shared-storage operations.

A PR exposing any high-impact family must document and test:

- compile-time and runtime gates;
- authenticated discovery/invocation scope;
- fixed allowlists and bounded inputs/outputs;
- confirmation or capability-grant requirements;
- dry-run/preview behavior;
- denied-access behavior;
- non-sensitive audit coverage;
- timeout, cancellation, cleanup, and rollback semantics.

## Manual Android smoke validation

For a versioned Termux deployment:

```bash
scripts/termux_deploy.sh status
sv status "$PREFIX/var/service/mcp_runtime"
curl -fsS http://127.0.0.1:8000/health
curl -fsS http://127.0.0.1:8000/ready
MCP_TEST_TOKEN="$(sed -n 's/^MCP__AUTH__STATIC_TOKEN=//p' "$HOME/.config/termux-mcp-edge/runtime.env")"
curl -sS \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp
curl -i -sS \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  http://127.0.0.1:8000/mcp
unset MCP_TEST_TOKEN
```

The evidence must identify the exact commit/artifact digest and verify authenticated discovery, no-response notification behavior, and representative allowed/denied tool calls.

## Stop conditions

Do not merge when any of these are true:

- exact-head CI is not green;
- required Security or Android validation is not green;
- browser-reachable routes lack auth, bounds, Host, or Origin protection;
- malformed and invalid request objects are misclassified;
- notifications receive JSON-RPC responses or dispatch request-only methods;
- unauthenticated clients can discover or invoke tools;
- high-impact tools are enabled by default;
- tests omit denied, boundary, cancellation, or recovery paths;
- documentation claims broader MCP interoperability than the implementation provides.
