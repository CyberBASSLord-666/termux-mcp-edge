# Security Best Practices for Termux MCP Edge

## Current Security Posture

Termux MCP Edge has two deliberate compile-time postures:

- The default feature set exposes the Axum `GET /health` endpoint and validates fail-closed startup authentication configuration.
- The optional `mcp-runtime` feature additionally exposes the staged `POST /mcp` JSON-RPC transport and its narrowly scoped tool registry.

The optional runtime is not a broad host-control surface. It validates exact `Host` and browser `Origin` allowlists before dispatch and exposes only the currently documented staged tools:

- `runtime_status`
- `platform_info`
- `android_status`
- `project_service_status`
- `list_directory`
- `read_file`
- `write_file`

Android platform control, shell fallback, arbitrary command execution, global process inventory, arbitrary service inspection, service mutation/control, package management, network mutation, and other high-impact controls remain disabled.

## Authentication and Startup Behavior

Startup requires `MCP__AUTH__STATIC_TOKEN` by default. Empty or whitespace-only values are rejected before the HTTP listener starts.

The only supported exception is explicit local development mode:

```bash
export MCP__AUTH__ALLOW_UNAUTHENTICATED_LOCALHOST_ONLY=true
export MCP__SERVER__HOST=127.0.0.1
```

This opt-in is rejected for non-loopback bind addresses. Do not use unauthenticated mode with tunnels, LAN exposure, reverse proxies, shared devices, or any remotely reachable deployment.

## Current Endpoint and Tool Surface

| Surface | Default build | `mcp-runtime` build |
|---|---:|---:|
| `GET /health` | Enabled | Enabled |
| `POST /mcp` staged transport | Disabled | Enabled |
| `runtime_status` / `platform_info` | Disabled | Read-only |
| `android_status` | Disabled | Read-only allowlisted metadata |
| `project_service_status` | Disabled | Read-only allowlisted project service metadata |
| `list_directory` | Disabled | Bounded and safe-rooted |
| `read_file` | Disabled | Bounded UTF-8 and safe-rooted |
| `write_file` | Disabled | Payload-bounded, safe-rooted, dry-run by default |
| Android control / command execution / high-impact controls | Disabled | Disabled |

## Transport Security

Browser-reachable MCP requests must match the configured exact transport allowlists:

```bash
export MCP__TRANSPORT__ALLOWED_HOSTS='["localhost:8000"]'
export MCP__TRANSPORT__ALLOWED_ORIGINS='["http://localhost:8000"]'
```

`MCP__TRANSPORT__ALLOW_MISSING_ORIGIN=true` is only appropriate for explicitly reviewed non-browser clients that cannot send `Origin`. It must not be used as a general browser compatibility bypass.

Transport validation occurs before JSON-RPC request dispatch. Rejected hosts or origins must not reach tool-call handling.

## Filesystem and Tool Safety Rules

Filesystem paths are canonicalized or resolved through existing parents and must remain inside configured safe roots. The implementation rejects relative paths, NUL bytes, explicit parent traversal, missing unsafe parents, and symlink escapes beyond a safe root.

The default safe root is deliberately narrow:

```text
/data/data/com.termux/files/home/mcp-files
```

Broad shared-storage roots such as `/storage/emulated/0` and `/sdcard` are not defaults. Empty safe-root lists, relative roots, and filesystem root `/` are rejected during configuration validation.

`read_file` and `write_file` are payload bounded. `write_file` defaults to preview behavior; mutation requires explicit `dry_run:false` and still passes safe-root and payload validation.

Read-only metadata tools must not expose environment values, raw secrets, persistent device identifiers, global process inventories, unrelated service state, or command output.

## Audit Counter Privacy

The staged runtime exposes in-memory aggregate audit counters through `runtime_status`. Counters retain stable tool names, allowed/denied totals, and low-cardinality reason codes only.

They must not retain raw paths, file contents, command arguments or output, environment names or values, bearer tokens, capability-token values, hostnames, usernames, Android identifiers, or arbitrary caller strings.

Audit counters provide evidence of gate decisions; they are not authorization and reset when the process restarts.

## Command and High-Impact Capability Boundaries

Command-policy and capability-token modules are inert policy scaffolding. Their presence does not enable process spawning, shell access, Android control, package/service/network mutation, or any high-impact MCP tool.

Any future command-capable or high-impact surface requires its own focused gate with compile-time and runtime opt-in, fixed allowlists, bounded execution, structured denial behavior, audit coverage, tests, and operator documentation.

## Dependency Advisory Policy

Dependency advisories must be resolved by one of these paths:

1. Remove unused vulnerable dependencies.
2. Upgrade to a patched compatible version.
3. Quarantine the affected feature from the compiled target while documenting the limitation.
4. Record an explicit accepted-risk exception only when there is no safe alternative.

Cargo, lockfile, or security-workflow changes must remain separate from unrelated runtime behavior changes and require exact-head CI and Security validation before merge.

## Deployment Hardening

- Bind to localhost unless a remote access path is explicitly required.
- Configure a strong bearer token before using tunnels or LAN access.
- Prefer a VPN-bound endpoint or named tunnel over raw port exposure.
- Keep exact Host and Origin allowlists minimal.
- Keep filesystem safe roots limited to dedicated project directories.
- Rotate tokens after suspected exposure.
- Keep CI, Security, Dependabot, and pinned GitHub Actions enabled.
- Validate the optional staged MCP surface before enabling it in a supervised service.

## Incident Response

If compromise is suspected:

1. Stop the runit service.
2. Rotate bearer tokens and tunnel credentials.
3. Inspect service and tunnel logs without copying secrets into issues or audit counters.
4. Recheck transport allowlists and filesystem safe-root configuration.
5. Review recent dependency, workflow, and runtime changes.
6. Redeploy only after the relevant exact-head CI and Security checks are green.
