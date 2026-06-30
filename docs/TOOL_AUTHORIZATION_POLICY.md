# MCP Tool Authorization Policy

This document defines the minimum authorization policy required before exposing MCP tools from this repository.

## Current Runtime Boundary

The current production line must remain conservative: `/health` may be exposed, but MCP tool discovery and tool invocation must remain unavailable until transport authentication, route protection, and tool authorization are merged and validated.

## Default Deny Rule

All MCP tools are denied by default. A tool may be exposed only when a focused pull request explicitly documents:

- the tool name and tool class;
- whether the tool is read-only or mutating;
- the data boundary it can read or modify;
- the feature gate or authorization rule that enables it;
- the tests or smoke notes that prove the default-deny path and allowed path behave as expected.

## Tool Risk Classes

### Class 0: health and metadata

Examples: health checks, static server metadata, version metadata.

Rules:

- May be unauthenticated only when it cannot disclose local paths, secrets, environment variables, hostnames, usernames, network topology, process lists, or dependency details useful for exploitation.
- Must not enumerate MCP tools unless the caller is authorized.

### Class 1: low-risk read-only tools

Examples: bounded status queries and reads from a narrow project-owned safe root.

Rules:

- Require authenticated MCP transport unless the tool is strictly local development-only and loopback-bound.
- Must enforce a narrow allowlist or safe root.
- Must reject path traversal, symlink escape, broad Android shared storage, app-private paths, credentials, and token material.
- Must include coverage for allowed and rejected reads.

### Class 2: mutating bounded tools

Examples: writing generated artifacts inside a project-owned output directory.

Rules:

- Require authenticated transport and explicit feature enablement.
- Must write only inside a configured safe root.
- Must reject overwrite, delete, chmod, rename, and recursive operations unless separately authorized.
- Must include tests or smoke validation for safe-root enforcement and denied mutations.

### Class 3: high-impact local tools

Examples: shell-like commands, package management, process control, rish/Shizuku actions, broad filesystem operations, Android shared-storage writes, network access, browser automation, credential handling, and anything capable of modifying host state outside a narrow project output directory.

Rules:

- Disabled by default in all builds.
- Require a dedicated feature gate or explicit authorization policy before registration.
- Require authenticated transport, Host validation, Origin validation, and fail-closed behavior before invocation.
- Must document command/path/network allowlists where applicable.
- Must include dedicated tests or smoke notes for blocked unauthorized access and the smallest allowed operation.
- Must be restored one tool class per pull request unless a narrower justification is documented.

## Registration Requirements

Tool registration must be authorization-aware:

1. Register no tools until the caller is authenticated or the runtime is in an explicitly documented loopback-only development mode.
2. Filter the tool list by authorization scope before returning tool discovery results.
3. Deny invocation when the requested tool is absent from the caller's authorized scope, even if the tool exists in the binary.
4. Avoid logging secrets, bearer tokens, session identifiers, command arguments containing credentials, or denied path values that may reveal sensitive host layout.

## Feature-Gate Requirements

A feature gate for Class 2 or Class 3 tools must define:

- the default value;
- the environment variable, CLI option, or build feature that enables it;
- the exact tool classes enabled by the gate;
- whether the gate is allowed in production;
- the validation evidence required before the gate can be used in a release.

Feature gates must not silently enable broad tool classes as a side effect of restoring MCP transport.

## Merge Blockers

A PR that exposes MCP tools is blocked if it:

- exposes any Class 1, Class 2, or Class 3 tool before authenticated transport exists;
- exposes Class 2 or Class 3 tools without explicit feature gating or authorization policy;
- returns high-impact tools in discovery for unauthorized clients;
- lacks safe-root or allowlist enforcement where filesystem, process, command, network, or browser operations are involved;
- lacks exact-head CI and Security success;
- lacks focused tests or smoke notes for denied and allowed paths.

## PR #21 Status

PR #21 remains blocked under this policy unless it is narrowed or replaced by staged PRs that satisfy the authorization, feature-gating, dependency, Host/Origin, and exact-head validation requirements.