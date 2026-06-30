# Production Readiness Checklist

This checklist defines the minimum production-readiness gate for the current conservative Termux runtime and for any future MCP transport restoration work.

## Current Supported Runtime

The current supported runtime is the Axum health-check service on `main`.

Production readiness for the current line means:

- `GET /health` is the only exposed HTTP route.
- MCP transport is not exposed.
- MCP tool discovery and invocation are not exposed.
- Filesystem, platform, command-capable, network, browser, package-manager, rish, and Shizuku-backed tools are not exposed.
- Startup fails closed unless a non-empty static bearer token is configured or explicit localhost-only development mode is enabled.
- Local unauthenticated development mode is rejected for non-loopback bind addresses.
- Filesystem safe roots remain narrow and do not default to Android shared storage.

## Required Merge Gate

Every production-readiness pull request must satisfy all of the following before merge:

1. Exact-head CI succeeds.
2. Exact-head Security succeeds.
3. The diff remains narrow and directly related to the stated remediation.
4. Dependency changes are absent, or dependency alerts are reviewed after the exact PR head is pushed.
5. Runtime claims in README, operations, validation, and security documentation match the compiled behavior.
6. No PR combines broad dependency restoration, transport exposure, and high-impact tool exposure in one change.

## Current Runtime Release Checklist

Before treating a build as releasable:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo build --release
```

Then verify the runtime locally:

```bash
curl -fsS http://127.0.0.1:8000/health
```

Expected response:

```text
ok
```

For Android deployment, also confirm:

- the runit service reads a token file created with restrictive permissions;
- the token file is present, non-empty, and not whitespace-only;
- the configured host is loopback unless an authenticated deployment path has been reviewed;
- the service is not exposed directly to the public internet;
- Android battery and process restrictions are configured for the intended service lifetime.

## MCP Restoration Readiness Gate

A future PR that restores MCP transport must not merge until it proves all of the following on the exact PR head:

- CI success.
- Security success.
- Dependency alerts are clear or documented as accepted exceptions.
- Authentication is enforced before MCP session or message handling.
- Unexpected `Host` headers are rejected.
- Unexpected browser `Origin` headers are rejected on browser-reachable routes.
- Unauthenticated development mode remains loopback-only.
- Unauthorized clients cannot discover tools.
- Unauthorized clients cannot invoke tools.
- MCP tool discovery has a smoke test.
- At least one permitted MCP tool call has a smoke test.
- Denied tool discovery or invocation paths have a smoke test or documented negative validation.

## High-Impact Tool Exposure Gate

High-impact tools include any tool that can:

- read or write files;
- list directories;
- execute commands or command-like platform actions;
- call package managers;
- access Android shared storage;
- use rish or Shizuku-backed privileges;
- make network requests;
- automate a browser;
- expose local device metadata beyond basic health information.

These tools must be disabled by default and protected by explicit feature gates, authorization scope, or an equivalent documented control before production exposure.

A PR that exposes high-impact tools must document:

1. which tools are exposed;
2. which clients can discover them;
3. which clients can invoke them;
4. what filesystem, command, network, or browser boundaries apply;
5. how denied access is tested;
6. how logs avoid leaking secrets, private paths, command arguments, tokens, cookies, and personal data.

## Stop Conditions

Do not merge when any of the following are true:

- CI or Security is missing, pending, cancelled, skipped, or failing for the exact PR head.
- The PR head changed after validation and has not been revalidated.
- The diff is broader than the stated production-readiness remediation.
- Documentation claims MCP readiness before MCP transport and tool validation exist.
- Dependency restoration reopens unresolved advisories.
- Browser-reachable routes lack documented Host and Origin protections.
- Command-capable or filesystem-capable tools are exposed without an explicit gate.
