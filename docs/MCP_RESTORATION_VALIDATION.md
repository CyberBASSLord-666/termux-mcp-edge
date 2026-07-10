# MCP Restoration Validation Plan

This document defines validation evidence for the current staged MCP transport and for any future capability expansion.

The default build remains a conservative Axum health/readiness service. The optional `mcp-runtime` build exposes authenticated staged MCP discovery and the documented allowlisted tool set. This plan must be updated whenever the live transport or authorization boundary changes.

## Required PR Shape

MCP work must remain staged through small, reviewable pull requests. A single PR must not combine broad dependency restoration, transport changes, high-impact tool exposure, and unrelated maintenance.

For future expansion, use this sequence:

1. Define the capability and threat model.
2. Add inert policy and validation primitives without runtime exposure.
3. Add or update authentication/authorization and audit boundaries.
4. Add disabled-by-default runtime wiring.
5. Add discovery and invocation tests for the smallest allowed surface.
6. Add operator documentation and recovery/rollback notes.

## Exact-Head Gate

Every MCP runtime PR must prove these checks on the exact PR head SHA:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets --all-features
cargo build --release
cargo build --release --features mcp-runtime
```

GitHub CI must succeed for the exact head. The Security workflow must also succeed when Cargo dependencies, the lockfile, or Security workflow inputs change. A path-filtered Security non-run is acceptable only when the PR documents that those inputs are unchanged.

## Dependency Gate

Before merge, dependency review must confirm one of the following:

- no dependency changes were made;
- dependency alerts remain clear after the exact PR head was pushed;
- any remaining advisory is documented as an accepted exception with scope, impact, mitigation, owner, and review date.

Restoring unused dependencies is not acceptable. Dependencies must correspond to compiled, tested code paths.

## Transport Authentication and Security Tests

Any PR that exposes or changes MCP transport must include automated or explicitly documented smoke coverage for these cases:

| Scenario | Expected result |
| --- | --- |
| Missing authorization in static-token mode | HTTP 401 before JSON-RPC handling. |
| Empty, malformed, oversized, or incorrect bearer token | HTTP 401 with no sensitive detail. |
| Valid bearer token | Request reaches Host/Origin validation and the permitted MCP path. |
| Unexpected `Host` header | Request is rejected. |
| Unexpected browser `Origin` header | Request is rejected on browser-reachable routes. |
| Loopback-only unauthenticated mode on loopback | Development-only startup and MCP access are allowed. |
| Loopback-only unauthenticated mode on non-loopback bind | Startup fails closed. |
| Debug/error output | Configured bearer values remain redacted. |

`/health` and `/ready` may remain unauthenticated only while their responses stay non-sensitive and contain no tool output.

## Tool Discovery Tests

Any PR that exposes or changes MCP tool discovery must prove:

1. unauthenticated clients cannot discover tools in static-token mode;
2. authenticated clients can discover only the intended tool set;
3. high-impact tools remain absent unless explicitly enabled;
4. discovery output does not reveal private filesystem paths, local tokens, command arguments, cookies, or Android account data.

## Tool Invocation Tests

Any PR that exposes or changes MCP tool invocation must prove:

1. unauthenticated invocation is rejected in static-token mode;
2. unknown or unauthorized tool names are rejected;
3. disabled high-impact tools cannot be invoked;
4. allowed tools enforce their configured boundaries;
5. failures do not leak secrets, private paths, command arguments, tokens, cookies, or personal data.

## High-Impact Tool Requirements

High-impact tools include command-capable actions, package management, process control, browser automation, network mutation, Android shared-storage operations, rish/Shizuku actions, device control, and any broad host mutation.

A PR exposing any high-impact tool must document and test:

- the feature gate and authorization scope that enable it;
- the default-disabled behavior;
- which authenticated clients can discover it;
- which authenticated clients can invoke it;
- path, command, network, browser, or platform boundaries;
- confirmation and dry-run/preview behavior;
- denied-access behavior;
- audit coverage that avoids sensitive data leakage;
- rollback and cancellation cleanup where feasible.

## Manual Android Smoke Validation

For Termux deployments, include a smoke-test note covering:

```bash
sv status mcp-server
curl -fsS http://127.0.0.1:8000/health
MCP_TEST_TOKEN="$(cat "$HOME/.termux_mcp_token")"
curl -sS \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp
unset MCP_TEST_TOKEN
```

The note must identify the exact build/commit and verify authenticated discovery plus at least one permitted tool call for the changed surface.

## Stop Conditions

Do not merge an MCP runtime PR when any of these are true:

- exact-head CI is not green;
- required Security validation is not green;
- dependency alerts are unresolved or unreviewed;
- browser-reachable routes lack Host or Origin protection;
- static-token mode does not enforce authentication before MCP handling;
- unauthenticated clients can discover or invoke tools;
- high-impact tools are enabled by default;
- tests or smoke notes do not cover denied access;
- documentation claims broader runtime capability than the compiled code provides.
