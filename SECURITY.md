# Security Policy

## Supported Runtime Scope

The supported runtime line has six explicit compile-time postures:

- The default build exposes operational health/readiness endpoints only.
- The optional `mcp-runtime` build exposes the staged `/mcp` transport and its documented allowlisted tool set.
- The optional `android-battery-status` build includes `mcp-runtime` and can expose one separately runtime-gated read-only battery tool.
- The optional `android-volume-status` build includes `mcp-runtime` and can expose one separately runtime-gated read-only volume-status tool.
- The optional `android-volume-control` build includes `mcp-runtime` and can expose one separately runtime-gated, preview-first, exact-grant-authorized volume-control tool.
- The optional `command-execution` build includes `mcp-runtime` and can expose one separately runtime-gated fixed-profile diagnostic tool.

The staged MCP route requires the configured static bearer token before JSON-RPC parsing, tool discovery, or tool invocation. The only exception is explicit unauthenticated localhost-only development mode, which startup validation restricts to a loopback bind.

The route implements the stable MCP 2025-11-25 Streamable HTTP lifecycle with bounded sessions, initialization gating, POST/GET/DELETE handling, and the specification-permitted HTTP 405 response for GET because server-initiated SSE and replay are not offered.

The baseline staged tools remain limited to `runtime_status`, `platform_info`, `android_status`, `project_service_status`, dry-run-first `create_directory`, dry-run-first `copy_file`, `hash_file`, `list_directory`, `path_metadata`, `read_file`, bounded literal `search_text`, and dry-run-first `write_file`. Directory mutation is separately default-disabled and requires one short-lived, target-bound, single-use `MCP-Capability-Grant`; explicit `dry_run:false` alone is denied. File copy accepts only one no-follow regular source up to 1 MiB, requires an absent safe-rooted destination with an existing parent, publishes mode `0600` without replacement, and returns no content. File hashing streams one no-follow regular file up to 16 MiB through SHA-256 and returns only the digest and byte count. Hashing, metadata, and search remain descriptor-relative and content-private. Separately built and explicitly enabled postures may add bounded `android_battery_status`, `android_volume_status`, `set_android_volume`, or `run_command_profile` under their dedicated contracts. The grant contract is defined in `docs/CREATE_DIRECTORY_CAPABILITY_GRANTS.md`; filesystem contracts are defined in `docs/SAFE_ROOT_DIRECTORY_CREATION.md`, `docs/SAFE_ROOT_FILE_COPY.md`, and `docs/SAFE_ROOT_FILE_HASHING.md`.

`run_command_profile` is supported only for the three fixed diagnostics of the exact running server binary. It accepts no raw command, program, argv, working directory, environment, stdin, timeout, or output-limit input. It uses a canonical safe-root cwd, cleared environment, null stdin, bounded streams, a hard deadline, process-group cleanup, zero-exit and UTF-8 requirements, and stable non-sensitive audit reasons. `set_android_volume` is supported only for the six fixed Termux:API streams, preview by default, and one exact single-use grant per live mutation with verification and recovery. Broader Android platform control, shells, arbitrary command execution, global process inventory, arbitrary service inspection, service mutation/control, package management, network mutation, and unrelated high-impact controls are not supported runtime surfaces.

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
- unauthorized clients cannot discover or invoke tools;
- high-impact tools are disabled by default and protected by explicit feature gates or authorization scope;
- allowed and denied paths are covered by tests or smoke notes on the exact PR head.

## Secret Handling

Logs, errors, debug formatting, test fixtures, audit counters, and documentation must not expose bearer tokens, session identifiers, private paths containing user names, SSH keys, API keys, cookies, or command arguments that contain credentials.

Use placeholders for examples, and redact sensitive values before adding logs or screenshots to issues and pull requests.

## Safe Disclosure Expectations

Security fixes should be staged as small pull requests with narrow diffs. Do not combine broad dependency restoration, transport exposure, and high-impact tool exposure in a single change unless a maintainer explicitly documents why the risk is acceptable and all gates are satisfied.
