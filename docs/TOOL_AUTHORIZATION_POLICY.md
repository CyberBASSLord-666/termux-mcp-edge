# MCP Tool Authorization Policy

This document defines the minimum authorization policy for MCP tools exposed by this repository.

## Current Runtime Boundary

The default build exposes operational health/readiness endpoints only. The optional `mcp-runtime` build exposes the staged MCP route after request authentication and transport Host/Origin validation.

In static-token mode, `/mcp` requires `Authorization: Bearer <configured-token>` before JSON-RPC parsing, discovery, or invocation. Explicit unauthenticated development mode is allowed only when startup confirms a loopback bind and request-time connection metadata proves the actual TCP peer is loopback; absent metadata and non-loopback peers fail closed.

The baseline staged registry contains `runtime_status`, `platform_info`, `android_status`, `project_service_status`, dry-run-first `create_directory`, dry-run-first `copy_file`, `find_paths`, `hash_file`, `list_directory`, `path_metadata`, `read_binary_file`, `read_binary_range`, `read_file`, `read_text_range`, literal `search_text`, and dry-run-first `write_file`. `create_directory` is a Class 2 mutation only when `dry_run:false`, its dedicated runtime gate is enabled, and one request-scoped single-use grant authorizes the authenticated principal/session and exact confined target. `write_file` is independently Class 2 only when `dry_run:false`, `MCP__FILE__WRITE_MUTATION_ENABLED=true`, static authentication and the capability key pair are configured, and one request grant binds the principal, session, safe-root identity, normalized target, exact UTF-8 content digest, `create` or `replace` disposition, and—for replacement—the exact existing file identity. It accepts at most 1 MiB, publishes the new file at mode `0600`, and returns a content- and path-free response within 16 KiB. Creation uses atomic no-replace publication and retains no artifact. Replacement accepts only a single-link regular target of at most 1 MiB and uses one irreversible exchange that preserves the displaced prior inode/content in the fixed bounded recovery quarantine. The result reports `recoveryArtifactRetained` without disclosing the artifact name. `copy_file` is also Class 2 only under its fixed one-file, 1 MiB, mode-`0600`, no-replace, content-private contract. `find_paths`, `hash_file`, `path_metadata`, `read_binary_file`, `read_binary_range`, `read_file`, `read_text_range`, and `search_text` remain Class 1 read-only tools with descriptor-relative confinement and content-private audits. Path discovery accepts one literal basename query and fixed kind/depth bounds, traverses at most 8,192 no-follow entries, and returns at most 512 content-free matches. Whole-file binary read accepts exactly one path and reads at most 1 MiB. Binary range read accepts exactly one path, offset, and length, and reads at most 256 KiB from a file up to 64 MiB. Text range read accepts one path, code-point-boundary byte offset, and 4-to-262,144-byte maximum from a UTF-8 file up to 64 MiB; it defers incomplete trailing code points and returns the next safe offset. All range reads reject offset-past-EOF or a detected size change and retain one verified no-follow regular-file descriptor. Binary tools return canonical padded base64; the text range returns valid UTF-8 only. Independent battery, volume-status, and fixed-command builds may additionally register their single bounded read-only tool only after the corresponding runtime flag is explicitly enabled. The independent volume-control posture registers one Class 3 tool only after its runtime/auth/key gates; preview is read-only, while live mutation additionally needs an exact single-use grant. No shell, arbitrary command execution, global process inventory, arbitrary service inspection, service mutation/control, package management, network mutation, or broader Android control is registered.

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

`set_android_volume` is Class 3 only under [`ANDROID_VOLUME_CONTROL.md`](ANDROID_VOLUME_CONTROL.md): independent compile/runtime gates, static authentication, exact enum and fresh maximum, preview default, principal/session/capability/stream/level-bound single-use grant, fixed two-argument execution, one non-queueing permit, status verification, automatic restoration, cancellation-independent recovery, stable redacted outcomes, and aggregate audit coverage.

`run_command_profile` is Class 1 only under its documented constraints: independent compile/runtime opt-in, the exact current server executable, three source-owned read-only argv profiles, a canonical safe-root cwd, empty environment, null stdin, bounded time and streams, two non-queueing permits, cancellation-safe process-group cleanup, UTF-8 zero-exit success, disabled discovery, and non-sensitive audit counters. Any other executable, placeholder, caller-selected value, broad inspection, credential use, shell/interpreter behavior, or mutation changes the risk class and requires a separate gate.

### Class 2: mutating bounded tools

Examples: writing generated artifacts inside a project-owned output directory.

Rules:

- Require authenticated transport and explicit feature enablement.
- Require an operation-scoped authorization grant when the mutation's impact exceeds the bearer principal's baseline preview posture; `create_directory` uses [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md), while `write_file` uses the independent contract in [`WRITE_FILE_CAPABILITY_GRANTS.md`](WRITE_FILE_CAPABILITY_GRANTS.md).
- Must default to dry-run or preview behavior where practical.
- Must write only inside a configured safe root.
- Must reject overwrite, delete, chmod, caller-directed rename, and recursive operations unless separately authorized. The `write_file` replacement disposition authorizes only replacement of the exact bound regular-file identity through its fixed transaction; it does not grant general rename, delete, or chmod authority.
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
2. Reflect the gate posture in discovery before returning tool schemas. The disabled `create_directory` and `write_file` mutation postures each constrain `dry_run` to `true`; each enabled posture advertises that its independent request grant is still required.
3. Deny invocation when the requested tool is absent from the caller's authorized scope, even if the tool exists in the binary.
4. Avoid logging secrets, bearer tokens, session identifiers, command arguments containing credentials, or denied path values that may reveal sensitive host layout.

The static token authenticates the complete MCP route before discovery or invocation. Live `create_directory` and `write_file` request grants are additive, header-only, independently capability-bound, and fail closed; a session ID or `dry_run:false` never substitutes for either. Exactly one bounded ASCII `MCP-Capability-Grant` header is accepted only on an active-session `tools/call` for a grant-aware tool. For `write_file`, schema validation, the disabled-gate denial, complete-response preflight, safe-root confinement, target classification, recovery-quarantine capacity, and exact binding validation all precede the first mutation; atomic grant consumption occurs immediately before publication work and remains consumed after any later failure.

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
