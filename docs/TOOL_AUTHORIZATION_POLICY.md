# MCP Tool Authorization Policy

This document defines the minimum authorization policy for MCP tools exposed by this repository.

## Current Runtime Boundary

The default build exposes operational health/readiness endpoints only. The optional `mcp-runtime` build exposes the staged MCP route after request authentication and transport Host/Origin validation.

In static-token mode, `/mcp` requires `Authorization: Bearer <configured-token>` before JSON-RPC parsing, discovery, or invocation. Explicit unauthenticated development mode is allowed only when startup validates an actually bound loopback listener and opaque request-time metadata from the accepted stream proves both a loopback peer and the exact validated local listener; absent metadata, non-loopback peers, and listener substitution fail closed.

The baseline staged registry contains `runtime_status`, `platform_info`, `android_status`, `project_service_status`, dry-run-first `create_directory`, dry-run-first `copy_file`, dry-run-first `trash_file`, `find_paths`, `hash_file`, `list_directory`, `path_metadata`, `read_binary_file`, `read_binary_range`, `read_file`, `read_text_range`, literal `search_text`, and dry-run-first `write_file`. `create_directory` is Class 2 only with its independent gate and exact principal/session/absent-target grant. `copy_file` is independently Class 2 only with its gate and a grant bound to both roots/paths, exact single-link source identity/content, and absent no-replace destination. `trash_file` is independently Class 2 only with `MCP__FILE__TRASH_FILE_MUTATION_ENABLED=true` and a grant bound to the exact single-link target identity/content and fixed recovery-retained posture. It moves the inode only into a bounded hidden recovery quarantine and exposes no purge or restore. `write_file` remains independently bound to exact content, disposition, and replacement identity. Every live filesystem mutation uses the shared fail-fast worker and process publication lock. `find_paths`, `hash_file`, metadata, binary/text reads, and search remain Class 1 descriptor-confined tools under fixed limits. Independent battery, volume-status, fixed-command, and volume-control postures remain separately compiled and runtime-gated. No shell, arbitrary command execution, raw deletion, recursive removal, global process inventory, arbitrary service inspection, service mutation/control, package management, network mutation, or broader Android control is registered.

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

`run_command_profile` is Class 1 only under its documented constraints: independent compile/runtime opt-in whose enabling `McpRouterBuilder` method is binary-crate-private while the public builder defaults disabled; an exact-name no-follow candidate matched by device/inode to independently opened `/proc/self/exe`; three source-owned read-only argv profiles that later spawn only `/proc/self/exe`; a no-follow descriptor-pinned non-root safe cwd; empty environment; null stdin; immutable 5-second/16 KiB stdout/4 KiB stderr maxima; two non-queueing permits; cancellation-safe process-group cleanup; UTF-8 zero-exit success; disabled discovery; and non-sensitive audit counters. Any other executable, placeholder, caller-selected value, broad inspection, credential use, shell/interpreter behavior, or mutation changes the risk class and requires a separate gate.

### Class 2: mutating bounded tools

Examples: writing generated artifacts inside a project-owned output directory.

Rules:

- Require authenticated transport and explicit feature enablement.
- Require an operation-scoped authorization grant when the mutation's impact exceeds the bearer principal's baseline preview posture: [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md), [`COPY_FILE_CAPABILITY_GRANTS.md`](COPY_FILE_CAPABILITY_GRANTS.md), [`TRASH_FILE_CAPABILITY_GRANTS.md`](TRASH_FILE_CAPABILITY_GRANTS.md), and [`WRITE_FILE_CAPABILITY_GRANTS.md`](WRITE_FILE_CAPABILITY_GRANTS.md) define the four independent filesystem contracts.
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
2. Reflect the gate posture in discovery before returning tool schemas. The disabled `create_directory`, `copy_file`, `trash_file`, and `write_file` mutation postures each constrain `dry_run` to `true`; each enabled posture advertises that its independent request grant is still required.
3. Deny invocation when the requested tool is absent from the caller's authorized scope, even if the tool exists in the binary.
4. Avoid logging secrets, bearer tokens, session identifiers, command arguments containing credentials, or denied path values that may reveal sensitive host layout.

The static token authenticates the complete MCP route before discovery or invocation. Live `create_directory`, `copy_file`, `trash_file`, and `write_file` request grants are additive, header-only, independently capability-bound, and fail closed; a session ID or `dry_run:false` never substitutes for any grant. Exactly one bounded ASCII `MCP-Capability-Grant` header is accepted only on an active-session `tools/call` for a grant-aware tool. Schema validation, disabled-gate denial, complete-response preflight, safe-root confinement, target classification, recovery constraints, and exact binding validation precede mutation; atomic grant consumption occurs immediately before the authorized namespace/publication work and remains consumed after any later failure.

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
