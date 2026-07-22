# Security Policy

## Supported Runtime Scope

The supported runtime line has seven governed compile-time postures: six least-privilege builds and one explicit aggregate:

- The default build exposes operational health/readiness endpoints only.
- The optional `mcp-runtime` build exposes the staged `/mcp` transport and its documented allowlisted tool set.
- The optional `android-battery-status` build includes `mcp-runtime` and can expose one separately runtime-gated read-only battery tool.
- The optional `android-volume-status` build includes `mcp-runtime` and can expose one separately runtime-gated read-only volume-status tool.
- The optional `android-volume-control` build includes `mcp-runtime` and can expose one separately runtime-gated, preview-first, exact-grant-authorized volume-control tool.
- The optional `command-execution` build includes `mcp-runtime` and can expose one separately runtime-gated fixed-profile diagnostic tool.
- The `full-suite` build composes all four optional feature families for one governed aggregate artifact. It still discovers exactly the 17 baseline tools when all optional runtime flags are off and exactly 21 only when all four flags are enabled independently.

`full-suite` is compile-time inclusion, not a master permission. Battery, volume-status, volume-control, and fixed-command runtime flags remain independent; filesystem and volume mutations retain their separate default-disabled gates and exact-operation request grants. Raw Cargo `--all-features` is retained for development compatibility and is not a public release posture.

The staged MCP route requires the configured static bearer token before JSON-RPC parsing, tool discovery, or tool invocation. The only exception is explicit unauthenticated localhost-only development mode: startup restricts the declared bind to loopback and request-time connection metadata must prove the actual TCP peer is loopback. Missing metadata and non-loopback peers fail closed; forwarded headers are not peer authority.

The route implements the stable MCP 2025-11-25 Streamable HTTP lifecycle with bounded sessions, initialization gating, POST/GET/DELETE handling, and a JSON-first posture whose GET requests return the specification-permitted HTTP 405. A separate default-disabled runtime setting permits only finite request-response SSE with session-owned, originating-stream replay under fixed stream, event, byte, and cursor limits; it does not provide broadcast or an unbounded server-message queue.

The baseline staged tools remain limited to `runtime_status`, `platform_info`, `android_status`, `project_service_status`, dry-run-first `create_directory`, dry-run-first `copy_file`, dry-run-first reversible `trash_file`, `find_paths`, `hash_file`, `list_directory`, `path_metadata`, `read_binary_file`, `read_binary_range`, `read_file`, `read_text_range`, bounded literal `search_text`, and dry-run-first `write_file`. Directory, copy, trash, and file-write mutation are independently default-disabled and each requires one short-lived, operation-bound, single-use `MCP-Capability-Grant`; explicit `dry_run:false` alone is denied. A copy grant binds the authenticated principal/session, both anchored roots and normalized paths, exact single-link source identity/size/high-resolution ctime/SHA-256, absent destination, and no-replace posture. A trash grant binds the exact single-link target identity, size, high-resolution ctime, SHA-256 content, and fixed recovery-retained posture. A write grant binds exact UTF-8 content, create-or-replace disposition, and exact old replacement identity. Grant wire payloads are fixed-shape and separately signed per family: create-directory and Android-volume grants encode their bounded authenticated binding fields, while copy, trash, and write use 65-byte opaque operation bindings. Live copy, trash, and writes are capped at 1 MiB. Copy and write-create publish mode `0600` with atomic no-replace; trash moves the exact inode with atomic no-replace into its separate private recovery quarantine; write-replace uses one irreversible exchange and preserves the displaced inode/content in a different reserved private quarantine. Each quarantine is mode `0700`, hidden from every MCP filesystem tool, and capped at 32 entries and 32 MiB per target parent. Copy, trash, and write results disclose neither content, path, digest, grant, nor recovery name. Directory creation returns its normalized safe-rooted path but never exposes grant material or descriptor/device/inode authority. Path discovery, binary and text-range reads, hashing, metadata, and text search retain their descriptor-relative fixed ceilings and content-private contracts. Separately built and explicitly enabled postures may add bounded `android_battery_status`, `android_volume_status`, `set_android_volume`, or `run_command_profile`. Grant contracts are defined in `docs/CREATE_DIRECTORY_CAPABILITY_GRANTS.md`, `docs/COPY_FILE_CAPABILITY_GRANTS.md`, `docs/TRASH_FILE_CAPABILITY_GRANTS.md`, and `docs/WRITE_FILE_CAPABILITY_GRANTS.md`; filesystem contracts remain in the corresponding `docs/SAFE_ROOT_*` documents.

`run_command_profile` is supported only for three fixed diagnostics of the attested already-loaded server image. Command enablement exists only in crate-private builders compiled into the package binary; every public library router hard-codes it disabled, including copy/all-filesystem constructors. Initialization proves an exact-name executable regular candidate and independently opened `/proc/self/exe` have equal device/inode identity, then spawns only `/proc/self/exe`. The first safe root is held by a no-follow directory descriptor, root aliases are rejected by device/inode, and children use `/proc/self/fd/<fd>` with the guard alive through execution. It accepts no raw command, program, argv, working directory, environment, stdin, timeout, or output-limit input and cannot exceed 5 seconds, 16 KiB stdout, or 4 KiB stderr. `set_android_volume` is supported only for the six fixed Termux:API streams, preview by default, and one exact single-use grant per live mutation with verification and recovery. Public directory-creation, copy, trash, file-write, and volume-control library APIs are preview-only; their live preparation and execution are crate-private and reachable only through grant-aware transport. One internal registry assigns globally unique wire codes to directory (`1`), write (`2`), volume (`3`), copy (`4`), and trash (`5`) request-grant families, and every ordered cross-family use fails without consuming the source grant. Broader Android platform control, shells, arbitrary command execution, global process inventory, arbitrary service inspection, service mutation/control, package management, network mutation, and unrelated high-impact controls are not supported runtime surfaces.

Request-grant replay, last-observed-clock, and capacity state is shared across every equivalent same-family/key-id/key/principal authority in one server process. The bounded registry and each authority namespace fail closed when unavailable, poisoned, or exhausted and expose no stable namespace identifier. This is not a cross-process guarantee: production must use one grant-consuming process per capability-key/principal domain, or an external atomic one-use coordinator, because separate processes retain independent replay state. Restart clears the in-memory state; rotate the key on restart when outstanding grants must be invalidated immediately.

## Reporting Security Issues

Do not open public issues for suspected vulnerabilities involving authentication bypass, token disclosure, filesystem escape, command execution, browser rebinding, local-network access, Android shared-storage exposure, or privilege-boundary bypass.

Report sensitive findings through GitHub private vulnerability reporting when available for this repository. If private reporting is unavailable, contact the maintainer out of band and include only the minimum detail needed to establish impact until a private channel is available.

## Required Triage Fields

Security reports should include:

- affected commit, tag, or pull request;
- deployment mode, including bind address and whether localhost-only development mode is enabled;
- exact route, tool, command, or file boundary involved;
- expected behavior and observed behavior;
- reproduction steps using placeholder secrets only;
- whether the finding requires browser access, local process access, LAN access, or authenticated MCP client access.

Reports must not include real bearer tokens, SSH keys, cookies, API keys, private file contents, or unrelated personal data from the Android device.

## Authentication Boundary

For static-token deployments, every request to `/mcp` must include:

```http
Authorization: Bearer <configured-token>
```

Authentication must run before transport validation, JSON-RPC parsing, discovery, or invocation. Missing, malformed, oversized, or incorrect credentials must return HTTP 401 with a non-sensitive response and `WWW-Authenticate: Bearer`.

`/health` and `/ready` remain unauthenticated operational probes and must not return secrets, raw configuration, private paths, or tool results.

Bearer values, capability HMAC keys, and issued grants must never appear in logs, debug output, errors, audit counters, tests, issue text, terminal transcripts, or screenshots.

## Dependency Advisory Gate

Dependency changes are blocked from merge until:

1. exact-head CI succeeds;
2. exact-head Security succeeds;
3. GitHub dependency alerts are reviewed after the change;
4. new advisories are fixed, removed, or explicitly documented as accepted exceptions;
5. unused dependency surfaces are removed instead of retained for future work.

A dependency may not be restored solely to support code paths that are not compiled or exposed in the current runtime.

## MCP Transport and Tool Exposure Gate

Any pull request that changes MCP transport, tool discovery, or tool invocation must satisfy the repository threat model and authorization policy before merge.

At minimum, it must prove:

- authenticated transport is enforced before MCP session or message handling;
- unexpected Host headers are rejected;
- unexpected Origin headers are rejected on browser-reachable routes;
- unauthenticated development mode remains loopback-only;
- localhost-only development rejects missing or non-loopback actual-peer connection metadata;
- unauthorized clients cannot discover or invoke tools;
- high-impact tools are disabled by default and protected by explicit feature gates or authorization scope;
- allowed and denied paths are covered by tests or smoke notes on the exact PR head.

## Secret Handling

Logs, errors, debug formatting, test fixtures, audit counters, and documentation must not expose bearer tokens, session identifiers, private paths containing user names, SSH keys, API keys, cookies, or command arguments that contain credentials.

Use placeholders for examples, and redact sensitive values before adding logs or screenshots to issues and pull requests.

## Safe Disclosure Expectations

Security fixes should be staged as small pull requests with narrow diffs. Do not combine broad dependency restoration, transport exposure, and high-impact tool exposure in a single change unless a maintainer explicitly documents why the risk is acceptable and all gates are satisfied.
