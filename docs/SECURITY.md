# Security Best Practices for Termux MCP Edge

## Current Security Posture

The current compiled runtime is an Axum HTTP service with a `/health` endpoint. MCP transport and MCP tool endpoints are not compiled into the current runtime.

This matters for risk assessment: the service should still be treated as network-facing software, but MCP tool-call risks are future-work risks until transport is deliberately restored.

## Authentication and Startup Behavior

Startup requires `MCP__AUTH__STATIC_TOKEN` by default. Empty or whitespace-only values are rejected before the HTTP listener starts.

The only supported exception is explicit local development mode:

```bash
export MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true
export MCP__SERVER__HOST=127.0.0.1
```

This opt-in is rejected for non-loopback bind addresses. Do not use unauthenticated mode with tunnels, LAN exposure, reverse proxies, shared devices, or any future tool-enabled runtime.

## Current Endpoint Surface

| Surface | Current state |
|---|---|
| `/health` | Enabled |
| MCP transport | Disabled |
| MCP filesystem tools | Disabled |
| Platform automation tools | Disabled |
| Command execution tools | Removed from source tree |

## Dependency Advisory Policy

Dependency advisories must be resolved by one of these paths:

1. Remove unused vulnerable dependencies.
2. Upgrade to a patched compatible version.
3. Quarantine the affected feature from the compiled target while documenting the limitation.
4. Record an explicit accepted-risk exception only when there is no safe alternative.

The current release line resolved visible dependency alerts by removing unused vulnerable dependency surfaces from the compiled runtime.

## Future MCP Transport Requirements

Any PR restoring MCP transport must include:

- A compatible, non-vulnerable transport dependency selection.
- CI and Security success on the exact PR head.
- Tests or smoke validation for MCP tool discovery.
- Tests or smoke validation for at least one tool call.
- Authentication and authorization documentation matching the implemented behavior.
- A clear threat model for every exposed tool.

## Filesystem and Tool Safety Rules

When filesystem tools are restored, path-taking code must canonicalize or safely resolve paths and enforce configured safe roots. Write access should remain disabled or tightly scoped until authorization and operator consent are implemented.

The default safe root is deliberately narrow and points to a dedicated Termux-home directory:

```text
/data/data/com.termux/files/home/termux-mcp-edge-files
```

Broad shared-storage roots such as `/storage/emulated/0` and `/sdcard` are not default safe roots. Empty safe-root lists, relative paths, and filesystem root `/` are rejected during configuration validation.

When platform or command-capable tools are restored, they must be feature-gated, documented, tested, and protected by explicit authorization policy. They must not be accidentally re-exported through broad module imports.

## Deployment Hardening

- Bind to localhost unless a remote access path is explicitly required.
- Configure a strong bearer token before using tunnels or LAN access.
- Prefer a VPN-bound endpoint or named tunnel over raw port exposure.
- Keep filesystem safe roots limited to dedicated project directories.
- Rotate tokens after suspected exposure.
- Keep CI, Security, and dependency scanning enabled.

## Incident Response

If compromise is suspected, stop the runit service, rotate bearer tokens, inspect logs, update dependencies, validate filesystem safe-root scope, and redeploy only after CI and Security are green.
