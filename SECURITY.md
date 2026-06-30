# Security Policy

## Supported Runtime Scope

The supported production line is the conservative Axum health-check runtime on `main`.

The current runtime intentionally exposes only `GET /health`. MCP transport, MCP tool discovery, filesystem tools, platform tools, shell-like actions, package-manager actions, network actions, browser automation, and rish/Shizuku-backed actions are not currently supported production surfaces.

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

## Dependency Advisory Gate

Dependency changes are blocked from merge until:

1. exact-head CI succeeds;
2. exact-head Security succeeds;
3. GitHub dependency alerts are reviewed after the change;
4. new advisories are fixed, removed, or explicitly documented as accepted exceptions;
5. unused dependency surfaces are removed instead of retained for future work.

A dependency may not be restored solely to support code paths that are not compiled or exposed in the current runtime.

## MCP Transport and Tool Exposure Gate

Any pull request that restores MCP transport, tool discovery, or tool invocation must satisfy the repository threat model and authorization policy before merge.

At minimum, it must prove:

- authenticated transport is enforced before MCP session or message handling;
- unexpected Host headers are rejected;
- unexpected Origin headers are rejected on browser-reachable routes;
- unauthenticated development mode remains loopback-only;
- unauthorized clients cannot discover or invoke tools;
- high-impact tools are disabled by default and protected by explicit feature gates or authorization scope;
- allowed and denied paths are covered by tests or smoke notes on the exact PR head.

## Secret Handling

Logs, errors, test fixtures, and documentation must not expose bearer tokens, session identifiers, private paths containing user names, SSH keys, API keys, cookies, or command arguments that contain credentials.

Use placeholders for examples, and redact sensitive values before adding logs or screenshots to issues and pull requests.

## Safe Disclosure Expectations

Security fixes should be staged as small pull requests with narrow diffs. Do not combine broad dependency restoration, transport exposure, and high-impact tool exposure in a single change unless a maintainer explicitly documents why the risk is acceptable and all gates are satisfied.
