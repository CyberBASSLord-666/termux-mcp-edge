# MCP Restoration Validation Plan

This document defines the validation evidence required before MCP transport, tool discovery, or tool invocation can be restored to the Termux MCP Edge runtime.

The current `main` runtime intentionally remains a conservative Axum health-check service. This plan is for future restoration PRs only and does not make the current runtime MCP-ready.

## Required PR Shape

MCP restoration must be staged through small, reviewable pull requests. A single PR must not combine broad dependency restoration, transport exposure, high-impact tool exposure, and documentation updates.

Recommended sequence:

1. Restore or update the minimum compatible MCP transport dependency surface.
2. Add authenticated transport wiring without exposing high-impact tools.
3. Add Host and Origin validation for browser-reachable routes.
4. Add tool discovery smoke coverage for a low-risk read-only surface.
5. Add one permitted tool-call smoke test.
6. Add high-impact tools only after feature-gating or authorization scoping is implemented and tested.

## Exact-Head Gate

Every restoration PR must prove these checks on the exact PR head SHA:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo build --release
```

GitHub Actions evidence must include successful `CI` and `Security` workflow runs for that exact head SHA. Missing, pending, cancelled, skipped, or stale runs are merge blockers.

## Dependency Gate

Before merge, dependency review must confirm one of the following:

- no dependency changes were made;
- dependency alerts remain clear after the exact PR head was pushed;
- any remaining advisory is documented as an accepted exception with scope, impact, mitigation, owner, and review date.

Restoring unused dependencies is not acceptable. Dependencies must correspond to compiled, tested code paths.

## Transport Security Tests

Any PR that exposes MCP transport must include automated or explicitly documented smoke coverage for these cases:

| Scenario | Expected result |
| --- | --- |
| Missing authorization | Request is rejected before MCP session handling. |
| Empty or malformed bearer token | Request is rejected. |
| Valid bearer token | Request reaches the permitted MCP path. |
| Unexpected `Host` header | Request is rejected. |
| Unexpected browser `Origin` header | Request is rejected on browser-reachable routes. |
| Loopback-only unauthenticated mode on loopback | Development-only startup is allowed. |
| Loopback-only unauthenticated mode on non-loopback bind | Startup fails closed. |

## Tool Discovery Tests

Any PR that exposes MCP tool discovery must prove:

1. unauthenticated clients cannot discover tools;
2. authenticated clients can discover only the intended tool set;
3. high-impact tools remain absent unless explicitly enabled;
4. discovery output does not reveal private filesystem paths, local tokens, command arguments, cookies, or Android account data.

## Tool Invocation Tests

Any PR that exposes MCP tool invocation must prove:

1. unauthenticated invocation is rejected;
2. unauthorized tool names are rejected;
3. disabled high-impact tools cannot be invoked;
4. allowed tools enforce their configured boundaries;
5. failures do not leak secrets, private paths, command arguments, tokens, cookies, or personal data.

## High-Impact Tool Requirements

High-impact tools include filesystem, command-capable, package-manager, browser automation, network, Android shared-storage, rish, Shizuku, and device-metadata actions.

A PR exposing any high-impact tool must document and test:

- the feature gate or authorization scope that enables it;
- the default-disabled behavior;
- which clients can discover it;
- which clients can invoke it;
- path, command, network, browser, or platform boundaries;
- denied-access behavior;
- audit logging that avoids sensitive data leakage.

## Manual Android Smoke Validation

For Termux deployments, restoration PRs should include a manual smoke-test note covering:

```bash
sv status mcp-server
curl -fsS http://127.0.0.1:8000/health
```

When MCP transport is restored, the note must also include the exact local command or client flow used to verify authenticated tool discovery and one permitted tool call.

## Stop Conditions

Do not merge an MCP restoration PR when any of these are true:

- CI or Security is not green on the exact PR head.
- Dependency alerts are unresolved or unreviewed.
- Browser-reachable routes lack Host or Origin protection.
- Authentication is not enforced before MCP session or message handling.
- Tool discovery is exposed without authorization.
- High-impact tools are enabled by default.
- Tests or smoke notes do not cover denied access.
- Documentation claims broader runtime capability than the compiled code provides.
