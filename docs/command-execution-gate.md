# Command execution gate design

## Purpose

This document defines the command-execution gate before any execution-capable MCP tool is enabled. Termux MCP Edge is intended for developers and advanced power users, so command execution can be a valid future capability, but only through explicit operator opt-in, fixed allowlists, bounded execution, and auditable decisions.

This design is a prerequisite. It does not enable command execution.

## Non-negotiable constraints

- Command execution is disabled by default.
- No shell interpolation.
- No arbitrary user-supplied command string.
- No global process inventory.
- No high-impact host or Android device-control actions.
- No implicit fallback from read-only tools into shell commands.
- No environment inheritance except an explicit allowlist.
- No writes outside configured safe-root policy.

## Capability enablement model

A future execution-capable build must require both compile-time and runtime opt-in:

1. Compile-time feature gate, for example `command-execution`.
2. Runtime configuration flag, for example `MCP__COMMAND__ENABLED=true`.
3. Non-empty command allowlist configuration or built-in conservative allowlist.
4. Audit event sink or counter path available before the first invocation.

If any requirement is missing, the transport must report command execution as disabled in `runtime_status` and reject command tool calls with a structured error.

## Command allowlist model

Allowed commands must be represented as named command profiles, not raw strings.

Each command profile should define:

- `profile_name`: stable public identifier exposed to MCP clients.
- `program`: absolute path or resolved fixed program name.
- `argv_template`: fixed argument vector with explicitly typed placeholders.
- `allowed_placeholders`: placeholder definitions with validators.
- `working_directory_policy`: safe-rooted or fixed directory.
- `environment_policy`: explicit key/value allowlist.
- `timeout_ms`: hard upper bound.
- `stdout_max_bytes`: hard upper bound.
- `stderr_max_bytes`: hard upper bound.
- `stdin_policy`: disabled by default; bounded static input only if later approved.
- `audit_gate_name`: stable audit gate identifier.

Example profile shape:

```text
profile_name = "cargo_metadata"
program = "cargo"
argv_template = ["metadata", "--format-version", "1", "--no-deps"]
working_directory_policy = "safe_root_required"
timeout_ms = 15000
stdout_max_bytes = 1048576
stderr_max_bytes = 65536
environment_policy = {}
```

## Argument handling

Argument construction must use fixed argv vectors. Placeholder expansion is allowed only when every placeholder has a validator.

Required validators:

- UTF-8 string length limit.
- No NUL bytes.
- No path traversal components for path placeholders.
- Safe-root resolution for path placeholders.
- Enum validation for mode-like values.
- Numeric min/max validation for numeric placeholders.

Rejected inputs must fail before process spawn.

## Working directory policy

Working directories must be one of:

- A fixed project directory compiled/configured by the operator.
- A resolved safe-rooted directory.

The runtime must reject:

- Relative working directories.
- Directories outside configured safe roots.
- Symlink escapes beyond a safe root.
- Missing directories unless the specific profile explicitly allows creation in a later gate.

## Environment policy

The default process environment is empty or minimal. The runtime must not inherit the server process environment wholesale.

Allowed environment variables must be profile-specific and explicit. Values must be fixed or derived from safe configuration, not from MCP request payloads unless a validator exists.

Denied by default:

- Tokens and credentials.
- Shell configuration.
- Android account or device identifiers.
- Full `PATH` inheritance.
- User home expansion.

## Execution bounds

Every invocation must enforce:

- Timeout.
- stdout byte cap.
- stderr byte cap.
- Maximum argv count.
- Maximum argument byte length.
- Optional concurrency limit.

On timeout, the child process must be terminated and the response must clearly indicate timeout without returning partial unbounded output.

## Audit requirements

Every attempted invocation must emit an audit event or equivalent metrics counters before returning.

Audit fields:

- Timestamp.
- Tool name.
- Command profile name.
- Gate name.
- Dry-run versus mutating/executing mode.
- Allowed or denied decision.
- Reason code.
- Timeout/limit metadata.
- Output byte counts, not raw output.

Audit logs must not include:

- Raw stdout or stderr.
- Secret values.
- Full environment.
- Raw file contents.
- Private host paths outside already-safe rooted paths.

## Required reason codes

At minimum:

- `command_execution_disabled`
- `profile_not_allowlisted`
- `invalid_argument_shape`
- `argument_validation_failed`
- `working_directory_outside_safe_root`
- `environment_key_not_allowlisted`
- `timeout_exceeded`
- `stdout_limit_exceeded`
- `stderr_limit_exceeded`
- `spawn_failed`
- `process_completed`

## MCP transport behavior

A future command tool must expose a closed schema with no additional properties.

The request should identify a command profile and structured arguments only. It must not accept a raw command string.

Responses must be structured:

- Success: exit status, bounded stdout/stderr strings, byte counts, duration, and profile name.
- Denied: structured JSON-RPC invalid-params or policy-denied response with reason code.
- Timeout: bounded timeout response with no unbounded partial output.

## Required tests before enabling runtime command execution

Unit tests:

- Allowlisted profile resolves to fixed argv.
- Unknown profile is denied.
- Raw command string cannot be submitted.
- Placeholder validation rejects NUL bytes, traversal, overlong values, and invalid enum values.
- Working directory must resolve inside safe root.
- Environment only contains allowlisted keys.
- stdout and stderr caps are enforced.
- Timeout path terminates the child process.
- Audit events are emitted for allowed and denied decisions.

Integration tests:

- MCP `tools/list` exposes command tool only when both compile-time and runtime gates are enabled.
- Disabled-by-default runtime status reports command execution disabled.
- Invalid requests fail before process spawn.
- A known safe profile executes with bounded output.
- Disallowed command attempts never spawn a process.

## Implementation sequence

1. Add inert command profile data types and validation tests.
2. Add command policy resolution and deny-by-default tests.
3. Add audit-event integration for denied decisions.
4. Add process runner behind feature gate, without MCP exposure.
5. Add MCP discovery and tool-call handling only after the runner and policy tests are green.
6. Add operator documentation and examples.

## Merge gate

No implementation PR may enable command execution unless:

- Exact-head CI is green.
- Security is green if dependencies, workflows, or lockfiles change.
- Tests prove disabled-by-default behavior.
- Tests prove no shell interpolation.
- Tests prove no arbitrary raw command string reaches process spawn.
- Documentation explains operator risk and opt-in requirements.
