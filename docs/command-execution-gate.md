# Fixed-profile command diagnostics

## Scope

Termux MCP Edge implements one deliberately narrow process-execution surface for read-only diagnostics of the server-owned, already-running executable image. It is not a shell, a generic command runner, a program launcher, or a path/argument templating system.

The public tool is `run_command_profile`. A caller can select only one reviewed profile identifier. The executable, complete argv vector, working directory, environment, stdin policy, timeout, output ceilings, and concurrency limit are owned by the server and cannot be overridden in an MCP request.

The Rust execution machinery is closed to the crate: command profiles, resolved handles, profile lookup, the raw execution client, and raw execution request/result types are not public API. Command enablement is structurally confined to the binary target. `src/main.rs` compiles the server module graph in the binary crate and alone can call the crate-private command switch on `McpRouterBuilder`; the library target exports one public builder that defaults command execution disabled and exposes no enabling method. No mintable command-authority token exists. Ordinary dependency and selected-workspace-member compile probes first build a valid safe consumer, then prove removed authority symbols, raw types, the binary-only switch, legacy router constructors, and their option/authority bundle types cannot be used.

Arbitrary command execution, shell evaluation, interpreters, caller-selected programs, caller-selected arguments, Android control, package or service mutation, network mutation, and other high-impact actions remain unavailable.

## Independent gates and binary-owned construction

Both operator gates are required:

1. Build the separate posture with `--features command-execution`.
2. Set `MCP__COMMAND__ENABLED=true` at runtime.

A `full-suite` build satisfies the command compile-time inclusion step, but it does not satisfy the independent runtime step. With `MCP__COMMAND__ENABLED` absent or false, `run_command_profile` remains hidden even when every optional feature is compiled. No request grant or other provider flag can enable it.

The feature includes `mcp-runtime`. The default build rejects `MCP__COMMAND__ENABLED=true` during startup. A command-capable build with the runtime flag absent or false hides `run_command_profile` from discovery and denies direct calls with `command_runtime_disabled` without spawning a process. With both opt-ins, executable and cwd-descriptor initialization must succeed while the common router builder is still fallible; otherwise startup returns `McpRouterBuildError::CommandClientUnavailable` before the bound listener can serve any request. Dependency embeddings and selected workspace consumers remain unable to enable the lane.

Example:

```bash
cargo build --release --locked --features command-execution
export MCP__COMMAND__ENABLED=true
```

Authentication, session lifecycle, Host/Origin policy, request concurrency, request timeout, and request-body limits still apply before tool dispatch.

## Fixed profile registry

| Profile | Exact argv | Timeout | stdout | stderr | Purpose |
|---|---|---:|---:|---:|---|
| `server_version` | `--version` | 5 s | 4 KiB | 1 KiB | Exact server version |
| `server_help` | `--help` | 5 s | 16 KiB | 1 KiB | Static CLI help |
| `execution_boundary` | `--self-check-command-boundary` | 5 s | 1 KiB | 1 KiB | Prove the child has empty environment, null stdin, and a non-root safe working directory |

At transport initialization, `std::env::current_exe()` must return an absolute path whose basename is exactly `termux-mcp-server`. The candidate is opened without following its final component and `/proc/self/exe` is opened independently; descriptor metadata must prove an executable regular candidate, a regular loaded image, and identical device/inode identity. Any failure returns the typed command-client construction error and aborts an enabled startup before the already-bound listener can serve or a child process can exist. Every profile then launches only `/proc/self/exe`, so later installation-path rename or replacement cannot redirect execution. No lookup through `PATH` occurs. The registry has no placeholders and no profile accepts caller data beyond its exact identifier.

`execution_boundary` is an internal CLI self-check used by native validation. It returns only `termux-mcp-command-boundary ok` and fails with one generic message if any boundary property is absent. It does not reflect the working directory, environment, or stdin target.

## Closed MCP schema

Discovery occurs only while both gates are enabled:

```json
{
  "name": "run_command_profile",
  "inputSchema": {
    "type": "object",
    "properties": {
      "profile": {
        "type": "string",
        "enum": ["server_version", "server_help", "execution_boundary"]
      }
    },
    "required": ["profile"],
    "additionalProperties": false
  }
}
```

The following request fields are rejected before policy resolution or spawn: `command`, `program`, `argv`, `workingDirectory`, `environment`, `stdin`, `timeout`, `stdoutLimit`, and `stderrLimit`. Unknown, missing, oversized, shell-shaped, or path-shaped profile identifiers are rejected.

## Process boundary

The shared bounded process supervisor provides all of these properties:

- exact absolute executable selected by the server;
- fixed complete argv selected by the profile;
- first already-canonicalized configured safe root opened once as a no-follow directory descriptor;
- filesystem-root aliases rejected by device/inode identity;
- child cwd selected through `/proc/self/fd/<fd>` while a descriptor guard remains alive through execution, so pathname rename or replacement cannot redirect it;
- completely cleared inherited environment;
- null stdin;
- separately piped and independently bounded stdout and stderr;
- immutable supervisor ceilings of 5 seconds, 16 KiB stdout, and 4 KiB stderr, independent of narrower profile limits and rejected before spawn if exceeded;
- output capacity grows with checked reservation only for bytes actually read—never from a selected ceiling—so allocation failure becomes a stable wait failure instead of a panic;
- no shell invocation or interpolation;
- isolated process group;
- hard operation deadline with a reserved nonzero cleanup window;
- immediate group termination on timeout, overflow, cancellation, process failure, or completion;
- authoritative direct-child reaping even after caller cancellation;
- stable failure if cleanup cannot be confirmed within the final deadline.

The command lane has its own non-queueing semaphore with two permits. If both permits are in use, another profile call fails immediately with `command_concurrency_limit_exceeded`; it does not wait behind running commands.

Successful output must be valid UTF-8. Invalid UTF-8, nonzero exit, timeout, overflow, spawn failure, wait failure, or program unavailability suppresses all child output and returns a stable non-sensitive reason code.

## Response contract

A successful response contains one bounded copy of each stream:

```json
{
  "profile": "server_version",
  "exitCode": 0,
  "stdout": "termux-mcp-server 0.6.0\n",
  "stderr": "",
  "stdoutBytes": 24,
  "stderrBytes": 0,
  "durationMilliseconds": 2
}
```

Only successful zero-exit profiles produce this shape. Failures use an MCP tool error with `command_profile_execution_failed` and one stable reason code; raw stdout, raw stderr, exit details, program paths, working-directory paths, environment values, and caller text are not returned.

## Stable reason codes

Policy and configuration:

- `command_feature_not_compiled`
- `command_runtime_disabled`
- `command_profile_missing_arguments`
- `command_profile_invalid_arguments`
- `command_profile_not_allowlisted`
- `command_safe_root_unavailable`

Execution:

- `command_program_unavailable`
- `command_spawn_failed`
- `command_wait_failed`
- `command_timeout`
- `command_stdout_limit_exceeded`
- `command_stderr_limit_exceeded`
- `command_program_failed`
- `command_output_invalid_utf8`
- `command_concurrency_limit_exceeded`

Success uses `command_profile_execution_allowed`.

## Audit privacy

Every resolved policy decision and execution outcome increments the existing in-memory aggregate audit counters. Events use:

- tool `run_command_profile`;
- gate `fixed_command_execution`;
- mode `read_only`;
- allowed or denied decision;
- stable reason code;
- the stable numeric profile ordinal only when an allowlisted profile was resolved.

Counters never retain the requested profile text, argv, program path, working directory, environment names or values, stdout, stderr, bearer material, session identifiers, or host paths. Disabled runtime decisions intentionally do not disclose whether the supplied profile identifier is allowlisted.

## Runtime posture reporting

`runtime_status` reports these fields independently:

- `commandExecutionCompiled`
- `commandExecution`
- `commandExecutionMode`, either `fixed_read_only_server_diagnostics` or `disabled`
- `arbitraryCommandExecution`, always `false`
- `highImpactTools`, always `false`

`android_status.command_execution_enabled` mirrors the effective fixed-profile runtime posture while `shell_fallback_enabled`, `android_control_enabled`, and `high_impact_controls_enabled` remain false.

## Validation gates

Unit and integration coverage proves:

- the registry is unique, fixed, and bounded;
- ordinary dependency and selected-workspace consumers can compile the safe public builder but cannot import or construct profiles, resolved handles, raw execution types, removed authority symbols, the binary-only command switch, legacy router constructors, or former option/authority bundle types; the public builder remains command-disabled;
- a wrong name, symlink, non-regular candidate, non-executable candidate, unavailable loaded image, or candidate/loaded device-inode mismatch rejects an enabled startup before the already-bound listener serves or any spawn is possible, while `/proc/self/exe` defeats post-initialization executable-path replacement after valid initialization;
- hard supervisor maxima reject an oversized timeout or stream limit before spawn, and checked buffer reservation cannot panic on an attacker-selected capacity;
- raw and injection-shaped identifiers are denied;
- all override fields fail before spawn;
- disabled discovery and direct-call behavior are fail-closed;
- the child receives fixed argv, a descriptor-pinned non-root safe working directory, empty environment, and null stdin;
- timeout, both output ceilings, nonzero exit, invalid UTF-8, cancellation cleanup, and command-specific concurrency are enforced;
- successful and denied decisions produce only non-sensitive counters;
- fixed mode is distinguished from arbitrary execution in runtime metadata.

The Android workflow builds the dedicated least-privilege artifact, `termux-mcp-server-aarch64-linux-android-command-execution`, as one of seven governed bundles. In the digest-pinned official ARM64 Termux container, `scripts/termux_command_emulated_gate.sh` performs exactly 29 MCP requests across enabled, combined executable/cwd replacement, and runtime-disabled phases. A separate wrong-name phase requires process failure with the typed non-sensitive command-client construction error, proves no service-start log or reachable health endpoint exists, and rejects token or path disclosure. The combined phase launches the server from `/`, replaces both configured pathnames after initialization, and calls `execution_boundary`; success proves the loaded executable inode and working-directory descriptor remain pinned. The workflow validates exact commit/version/CI/Security/Android provenance, both artifact digests and size bounds, environment posture, and every v2 validation boolean, including `wrongExecutableNameRejectedBeforeServing`. The sanitized report conforms to `docs/command-emulated-evidence-schema-v2.json`; command v1 is historical-only.

This native gate is deterministic and does not require a long observation window. It is development evidence, not by itself a physical-device release qualification.

## Expansion rule

Adding a profile is a security-sensitive public-surface change. A profile is not eligible if it accepts placeholders, evaluates code, reads broad host state, mutates any state, requires credentials, uses a shell/interpreter, or can escape the configured safe root. Such work requires a separate capability gate and threat review rather than broadening `run_command_profile`.
