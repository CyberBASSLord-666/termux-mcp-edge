# MCP Runtime Validation Plan

This document defines validation evidence for the stable MCP transport around the current staged tool surface and for future protocol or capability expansion.

The default build remains a conservative Axum health/readiness service. The optional `mcp-runtime` build exposes authenticated stable MCP 2025-11-25 Streamable HTTP handling and the documented allowlisted tool set. It uses bounded session-backed lifecycle state, declines optional SSE with HTTP 405 by default, and offers a separately configured finite SSE response/resumption posture.

## Required PR shape

MCP work must remain staged through small, reviewable pull requests. Do not combine broad dependency restoration, protocol/lifecycle changes, high-impact tool exposure, and unrelated maintenance.

For protocol or capability expansion:

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
cargo build --release --features android-battery-status
cargo build --release --features android-volume-status
cargo build --release --features command-execution
```

CI must succeed on the exact head. Security must succeed when Cargo, lockfile, or Security-workflow inputs change. Android cross-compilation must succeed for the default, `mcp-runtime`, `android-battery-status`, `android-volume-status`, and `command-execution` AArch64 postures when Rust source, toolchain, dependency, workflow, cross-compilation, or deployment changes can affect device artifacts.

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
- valid non-object JSON, batch arrays, missing/wrong `jsonrpc`, missing/non-string method, invalid ID types, and non-object params return `-32600 Invalid Request`;
- `jsonrpc` is exactly `"2.0"`;
- MCP request IDs are non-null strings or integer numbers;
- MCP params are objects when present;
- valid client success/error responses are distinguished from requests and notifications;
- notifications and client responses receive HTTP 202 with an empty body and no JSON-RPC response object;
- notification-shaped request methods are not dispatched and cannot mutate state;
- unsupported but valid notifications receive HTTP 202 without dispatch;
- valid request IDs are preserved in request errors, while invalid IDs are never reflected.

The adopted 2025-11-25 schema defines a single JSON-RPC message rather than a batch, so arrays remain a documented invalid-request compatibility decision. This server does not issue JSON-RPC requests to clients; syntactically valid client responses are accepted and discarded without retained correlation state.

## Stable protocol and transport regression gate

The implemented MCP 2025-11-25 postures must continue to prove:

- initialization as the first client/server interaction;
- protocol-version and capability negotiation;
- receipt of `notifications/initialized` before normal operation;
- a single MCP endpoint with POST, GET, and DELETE handling;
- compliant POST `Content-Type` and explicit `Accept` support for JSON and `text/event-stream` responses;
- exact browser `Origin` protection before message handling;
- the `MCP-Protocol-Version` request-header contract after initialization;
- compliant notification and response status/body behavior;
- cryptographically random visible-ASCII UUID sessions, bounded to 64 records with 30-minute idle expiry, explicit DELETE termination, and no retained client initialize metadata;
- independent pending/active state across concurrent sessions;
- ping before activation, request timeout/cancellation-safe cleanup, process-shutdown state reset, HTTP 404 reinitialization behavior, and multiple-client isolation;
- GET with `Accept: text/event-stream` returning HTTP 405 in the default posture without creating replay state;
- opt-in finite SSE responses contain an empty primer plus one terminal response, use globally unique stream-derived event IDs, and fall back to JSON above the 128 KiB event ceiling;
- cursor-bearing GET replays only later events from the exact session and originating stream; malformed/duplicate/oversized cursors, unknown/cross-session cursors, oldest-first eviction, terminal cursors, termination, and expiry are covered;
- replay remains bounded to 8 streams, 2 events per stream, and 256 KiB per session, with no broadcast or long-lived server queue;
- rejection of batch arrays, consistent with the selected stable schema's single-message transport body.

The target contracts are the official [MCP 2025-11-25 specification](https://modelcontextprotocol.io/specification/2025-11-25), [lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle), and [Streamable HTTP transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports) documents.

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
5. no-argument tools accept only omitted arguments or an empty object;
6. every advertised closed schema rejects null/scalar/array shapes, missing required fields, wrong types, and unknown fields;
7. rejected mutating arguments cannot create or alter files;
8. enabled tools enforce safe roots, bounds, dry-run/mutation rules, and audit privacy;
9. failures do not leak secrets, private paths, raw I/O errors, serde diagnostics, rejected values, command arguments, tokens, or personal data.

## High-impact tool requirements

High-impact tools include arbitrary or mutating command execution, new executable authority beyond the fixed server diagnostics, package management, process/service control, Android/device control, broad filesystem mutation, network mutation, browser automation, credential handling, and shared-storage operations.

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
MCP_RESPONSE_HEADERS="$(mktemp)"
trap 'rm -f "$MCP_RESPONSE_HEADERS"; unset MCP_TEST_TOKEN MCP_SESSION_ID MCP_RESPONSE_HEADERS' EXIT

curl -sS -D "$MCP_RESPONSE_HEADERS" \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  --data '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-smoke-test","version":"1.0.0"}}}' \
  http://127.0.0.1:8000/mcp | jq -e '.result.protocolVersion == "2025-11-25"'

MCP_SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$MCP_RESPONSE_HEADERS")"
test -n "$MCP_SESSION_ID"

test "$(curl -sS -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "MCP-Session-Id: ${MCP_SESSION_ID}" \
  --data '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  http://127.0.0.1:8000/mcp)" = 202

curl -sS \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "MCP-Session-Id: ${MCP_SESSION_ID}" \
  --data '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp | jq -e '.result.tools | length == 16'

test "$(curl -sS -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Accept: text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "MCP-Session-Id: ${MCP_SESSION_ID}" \
  http://127.0.0.1:8000/mcp)" = 405

test "$(curl -sS -o /dev/null -w '%{http_code}' -X DELETE \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "MCP-Session-Id: ${MCP_SESSION_ID}" \
  http://127.0.0.1:8000/mcp)" = 204

rm -f "$MCP_RESPONSE_HEADERS"
unset MCP_TEST_TOKEN MCP_SESSION_ID MCP_RESPONSE_HEADERS
trap - EXIT
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
