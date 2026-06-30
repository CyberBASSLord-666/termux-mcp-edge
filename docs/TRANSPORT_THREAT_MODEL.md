# MCP Transport Threat Model

This document defines the minimum security posture required before restoring MCP transport on this repository's production line.

## Current Runtime Boundary

The current `main` branch exposes only the Axum `/health` endpoint. MCP transport, MCP tool discovery, filesystem tools, platform tools, shell-like actions, network actions, and browser automation actions must remain unavailable until they are restored through small validated pull requests.

## Assets to Protect

- Termux home data and any configured file safe roots.
- Android shared storage and external media paths.
- Device-local credentials, tokens, SSH keys, API keys, cookies, and app-private files.
- Local network resources reachable from the Android device.
- Host shell, Shizuku/rish-backed privilege boundaries, package-manager state, and process controls.
- MCP client trust boundaries and tool invocation integrity.

## Primary Threats

### Browser rebinding and ambient browser access

A browser page can try to reach a local listener through `localhost`, `127.0.0.1`, `::1`, LAN IPs, DNS rebinding, or redirects. MCP transport routes must not be reachable from arbitrary browser origins.

Required controls before transport restoration:

- Reject unexpected `Host` headers.
- Reject unexpected `Origin` headers on browser-reachable routes.
- Reject missing or malformed authentication on non-local access paths.
- Keep unauthenticated development mode loopback-only.
- Add tests or smoke coverage proving hostile Host/Origin combinations fail closed.

### Cross-client MCP confusion

A valid local client should not allow another process, browser tab, or remote peer to reuse transport state or invoke tools without authorization.

Required controls:

- Authentication must be checked before any MCP session or message handling.
- Session identifiers must not be enough to authorize tool calls by themselves.
- Transport state must be scoped to the authenticated client context.
- Logs must not expose bearer tokens or session secrets.

### High-impact tool exposure

Files, shell-like commands, package management, process control, network access, browser automation, and privileged Android actions can cross from helpful automation into full device compromise.

Required controls:

- Restore one low-risk read-only tool first.
- Keep write, delete, shell, rish/Shizuku, package-manager, process, network, and browser-automation tools behind explicit feature gates or authorization policy.
- Require dedicated tests or smoke notes for each new exposed tool class.
- Keep configured filesystem safe roots narrow by default.

### Dependency and protocol drift

Transport dependencies and MCP protocol crates may change API or security posture over time.

Required controls:

- Exact-head CI and Security must pass for the transport PR.
- Dependency alerts must be checked after any dependency restoration.
- The selected transport dependency and version must be documented in the PR body.
- Broad dependency restoration without compiled runtime usage is not acceptable.

## Minimum Transport Restoration Sequence

1. Add or restore a transport dependency only after dependency alerts are clear.
2. Add authentication middleware and Host/Origin enforcement before registering MCP routes.
3. Add route-level tests or smoke coverage for accepted and rejected Host/Origin/auth combinations.
4. Register MCP transport with no high-impact tools exposed.
5. Add MCP initialization and tool-discovery smoke coverage.
6. Add one read-only low-risk tool with explicit safe-root or data-boundary tests.
7. Add higher-impact tools only after feature gates or an explicit authorization policy are merged.

## Merge Blockers for Broad Restoration PRs

A PR is blocked if it:

- Restores MCP transport and multiple tool classes in one broad change.
- Exposes filesystem, system, shell, rish/Shizuku, package-manager, network, or browser automation actions without feature gates or authorization policy.
- Lacks Host and Origin protections for browser-reachable transport routes.
- Lacks exact-head CI and Security success.
- Reintroduces dependency advisories or unused dependency surface.
- Claims production readiness without smoke validation against the exact PR head.

## PR #21 Status

PR #21 remains blocked under this threat model unless it is narrowed or replaced with staged PRs that satisfy the controls above.
