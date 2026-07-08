# Staged capability gates

The MCP runtime expands capability only through small, current-base pull requests with explicit scope, tests, and audit coverage. Every gate must preserve the existing default-deny posture unless the gate explicitly changes that posture and the change is covered by tests.

## Current baseline

Enabled staged tools:

- `runtime_status`
- `platform_info`
- `list_directory`
- `read_file`
- `write_file` with dry-run by default and explicit safe-rooted mutation only when `dry_run: false`

Still disabled:

- Android API access
- Shell and command execution
- Process listing and arbitrary service inspection
- High-impact device or host controls

## Gate 1: non-sensitive platform metadata

Status: implemented.

Allowed:

- OS
- Architecture
- Platform family
- Available parallelism
- Package version

Denied:

- Environment variables
- Usernames and hostnames
- Device identifiers
- Filesystem paths beyond existing safe-rooted filesystem tools
- Process lists
- Shell access
- Android API calls

Required coverage:

- Discovery test
- Tool-call success test
- Argument-rejection test
- Runtime status test proving Android/platform control, command execution, and high-impact tools remain disabled

## Gate 2: Android read-only status

Allowed only as a separate PR after Gate 1.

Potential scope:

- Explicitly allowlisted Android/Termux status fields useful for local diagnostics
- Read-only values only
- Structured output only

Denied:

- Contacts, SMS, notifications, accounts, location, camera, microphone, accessibility state, installed package inventory, persistent device IDs, and user secrets
- Shell fallback
- Any mutation or device-control action

Required before merge:

- Written allowlist and denylist
- Tests proving denied fields are absent
- Runtime status metadata showing read-only Android status only
- No new dependency unless Security passes exact-head audit

## Gate 3: project-owned service state

Allowed only as a separate PR after Android read-only status.

Potential scope:

- Status of explicitly allowlisted project-owned services
- Structured service health fields

Denied:

- Global process listing
- Arbitrary PID inspection
- Command execution
- Reading unrelated process command lines or environment

Required before merge:

- Service allowlist
- Tests proving unrelated services/processes are not exposed
- Structured unsupported-service errors
- Audit event for each service-status query if audit logging has landed

## Gate 4: command execution

Highest-risk runtime capability. Keep disabled until earlier gates are stable and audit logging exists.

The detailed gate design is maintained in [`command-execution-gate.md`](command-execution-gate.md).

Required before implementation:

- Explicit command allowlist
- Fixed argv vectors only; no shell interpolation
- Timeout enforcement
- Output byte limits
- Working-directory safe-root policy
- Environment allowlist
- Audit event per invocation
- Tests for injection attempts, disallowed commands, timeout, output cap, environment filtering, and safe-root violations

## Gate 5: high-impact controls

Examples:

- Package installation or removal
- Service restart or stop
- File deletion outside the staged safe-root write policy
- Network or device configuration changes
- Any Android device-control action

The detailed threat model is maintained in [`high-impact-controls-threat-model.md`](high-impact-controls-threat-model.md).

Required before implementation:

- Dedicated threat model
- Explicit capability token or confirmation design
- Dry-run or preview mode where possible
- Full audit trail
- Rollback plan where feasible
- Security review before merge

## Cross-cutting audit coverage

Before any mutating or command-capable gate expands further, add a dedicated audit log primitive that records:

- Timestamp
- Tool name
- Gate name
- Dry-run versus mutating mode
- Allowed or denied decision
- Non-sensitive reason code
- Size/limit metadata where relevant

Audit logs must not include secrets, raw file contents, environment values, command output, or private filesystem paths beyond already-safe rooted paths.
