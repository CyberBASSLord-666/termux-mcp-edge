# MCP Tool Authorization Policy

This document defines the minimum authorization policy for MCP tools exposed by this repository.

## Current Runtime Boundary

The default build exposes operational health/readiness endpoints only. The optional `mcp-runtime` build exposes the staged MCP route after request authentication and transport Host/Origin validation.

In static-token mode, `/mcp` requires `Authorization: Bearer <configured-token>` before JSON-RPC parsing, discovery, or invocation. Explicit unauthenticated development mode is allowed only when startup validation confirms a loopback bind.

The baseline staged registry contains `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `list_directory`, `path_metadata`, `read_file`, literal `search_text`, and dry-run-first `write_file`. `path_metadata` is read-only, safe-rooted, descriptor-relative, content-free, and limited to regular-file/directory kind, file size, and optional modification time; host identifiers and permission internals are excluded. `search_text` is read-only, safe-rooted, descriptor-relative, fixed-limit, content-free location search; query text is neither executed nor audited. Independent `android-battery-status`, `android-volume-status`, and `command-execution` builds may additionally register their single bounded read-only tool only after the corresponding runtime flag is explicitly enabled. No Android or audio control, shell, arbitrary command execution, global process inventory, arbitrary service inspection, service mutation/control, package management, network mutation, or high-impact tool is registered.

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
- Must not enumerate MCP tools unless the caller is authenticated or the runtime is in explicitly validated loopback-only development mode.

### Class 1: low-risk read-only tools

Examples: bounded status queries and reads from a narrow project-owned safe root.

Rules:

- Require authenticated MCP transport unless the tool is strictly local development-only and loopback-bound.
- Must enforce a narrow allowlist or safe root.
- Must reject path traversal, symlink escape, broad Android shared storage, credentials, and token material.
- Must include coverage for allowed and rejected reads.

`android_battery_status` is Class 1 only under its documented constraints: authenticated transport, separate compile/runtime opt-in, one fixed absolute Termux:API executable, no caller arguments or inherited environment, bounded time and output, a strict normalized field allowlist, disabled discovery, stable non-sensitive failures, and aggregate audit coverage. Expanding it to caller-selected commands, additional Android APIs, identifiers, broad device data, or mutation changes the risk class and requires a separate gate.

`android_volume_status` is Class 1 only under its documented constraints: independent compile/runtime opt-in, fixed zero-argument `termux-volume` status execution, cleared environment, bounded time/output, exact six-stream parsing, canonical output, disabled discovery, stable failures, the shared hardened provider supervisor, and aggregate audit coverage. Passing any argument, selecting a stream/level, or otherwise reaching volume mutation changes the risk class and requires a separate high-impact gate.

`run_command_profile` is Class 1 only under its documented constraints: independent compile/runtime opt-in, the exact current server executable, three source-owned read-only argv profiles, a canonical safe-root cwd, empty environment, null stdin, bounded time and streams, two non-queueing permits, cancellation-safe process-group cleanup, UTF-8 zero-exit success, disabled discovery, and non-sensitive audit counters. Any other executable, placeholder, caller-selected value, broad inspection, credential use, shell/interpreter behavior, or mutation changes the risk class and requires a separate gate.

### Class 2: mutating bounded tools

Examples: writing generated artifacts inside a project-owned output directory.

Rules:

- Require authenticated transport and explicit feature enablement.
- Must default to dry-run or preview behavior where practical.
- Must write only inside a configured safe root.
- Must reject overwrite, delete, chmod, rename, and recursive operations unless separately authorized.
- Must include tests or smoke validation for safe-root enforcement, size limits, denied mutations, and explicit allowed mutation.

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

1. Register or return no tools to an unauthenticated caller.
2. Filter the tool list by authorization scope before returning discovery results when scoped authorization is introduced.
3. Deny invocation when the requested tool is absent from the caller's authorized scope, even if the tool exists in the binary.
4. Avoid logging secrets, bearer tokens, session identifiers, command arguments containing credentials, or denied path values that may reveal sensitive host layout.

The current static-token stage authenticates the complete MCP route before discovery or invocation. Future per-tool scopes must be additive and fail closed.

## Feature-Gate Requirements

A feature gate for Class 2 or Class 3 tools must define:

- the default value;
- the environment variable, CLI option, or build feature that enables it;
- the exact tool classes enabled by the gate;
- whether the gate is allowed in production;
- the validation evidence required before the gate can be used in a release.

Feature gates must not silently enable broad tool classes as a side effect of restoring MCP transport.

## Merge Blockers

A PR that exposes or changes MCP tools is blocked if it:

- exposes any Class 1, Class 2, or Class 3 tool before authenticated transport exists;
- exposes Class 2 or Class 3 tools without explicit feature gating or authorization policy;
- returns tools or tool results to unauthenticated clients;
- lacks safe-root or allowlist enforcement where filesystem, process, command, network, or browser operations are involved;
- lacks exact-head CI success;
- lacks Security validation when dependencies, lockfiles, or security workflow inputs change;
- lacks focused tests or smoke notes for denied and allowed paths.
