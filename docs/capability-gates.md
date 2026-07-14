# Staged capability gates

The MCP runtime expands capability only through small, current-base pull requests with explicit scope, tests, and audit coverage. Every gate must preserve the existing default-deny posture unless the gate explicitly changes that posture and the change is covered by tests.

## Current baseline

Enabled staged tools:

- `runtime_status`
- `platform_info`
- `android_status`
- `project_service_status`
- `create_directory` for exactly one absent safe-rooted directory, dry-run by default, fixed mode `0700`, and atomic no-replace publication
- `list_directory`
- `path_metadata` for one descriptor-relative regular-file or directory metadata result without host identifiers
- `read_file`
- `search_text` for bounded literal UTF-8 location search without content excerpts
- `write_file` with dry-run by default and explicit safe-rooted mutation only when `dry_run: false`

Separately gated read-only tool:

- `android_battery_status` only in an `android-battery-status` build with `MCP__ANDROID__BATTERY_STATUS_ENABLED=true`
- `android_volume_status` only in an `android-volume-status` build with `MCP__ANDROID__VOLUME_STATUS_ENABLED=true`
- `run_command_profile` only in a `command-execution` build with `MCP__COMMAND__ENABLED=true`; this is a closed set of fixed read-only server diagnostics, not arbitrary command execution

Current audit visibility is aggregate and in-memory. The staged runtime exposes backend-neutral `auditCounters` through `runtime_status` for the currently wired status and filesystem surfaces. These counters are intentionally not retained request logs and store only stable tool names, gate names, modes, reason codes, and allowed or denied counts.

Still disabled:

- Android platform or audio control beyond read-only allowlisted status and optional battery/volume telemetry
- Shell, arbitrary commands, caller-selected executables/argv, and all command mutation
- Global process listing and arbitrary service inspection
- Service mutation or control
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

Status: implemented for static read-only allowlisted Android/Termux status metadata and separately gated read-only battery and volume telemetry. Android and audio control remain disabled.

Baseline `android_status` scope:

- Explicitly allowlisted Android/Termux status fields useful for local diagnostics
- Read-only values only
- Structured output only
- No Android API access or control surface
- No shell fallback

Optional battery scope:

- Separate `android-battery-status` compile-time feature, which includes `mcp-runtime`
- Separate `MCP__ANDROID__BATTERY_STATUS_ENABLED=true` runtime opt-in, defaulting to disabled
- Direct execution of one fixed absolute Termux:API program with zero arguments, null stdin, and a cleared inherited environment
- Five-second normal-operation budget with a reserved cleanup window, 16 KiB stdout limit, and 4 KiB stderr limit
- Single cancellation-safe supervisor with isolated process-group termination, immediate overflow handling, bounded pipe completion, and authoritative direct-child reaping; cleanup-reserve exhaustion overrides every primary result with a stable wait failure
- Strict normalized battery-field allowlist with unknown fields, technology/vendor text, identifiers, raw output, and stderr discarded
- Hidden discovery while disabled and stable non-sensitive errors for disabled or unavailable states
- Aggregate allowed/denied audit counters using stable reason codes only
- Native ARM64 official-Termux execution with a fixed-path API fixture, endless-output, pipe-holder, and client-cancellation cleanup checks in CI

Optional volume scope:

- Separate `android-volume-status` compile-time feature, which includes `mcp-runtime`
- Separate `MCP__ANDROID__VOLUME_STATUS_ENABLED=true` runtime opt-in, defaulting to disabled
- Direct execution of only `/data/data/com.termux/files/usr/bin/termux-volume` with zero arguments, null stdin, fixed `/` working directory, and a cleared inherited environment
- Five-second normal-operation budget with a reserved cleanup window, 8 KiB stdout limit, and 4 KiB stderr limit
- The same cancellation-safe provider supervisor, process-group isolation, immediate overflow termination, pipe completion, and authoritative direct-child reaping used by battery telemetry
- Exact six-stream and exact-field parser with integer/range validation and canonical `alarm`, `call`, `music`, `notification`, `ring`, `system` output order
- Rejection rather than reflection of unknown, duplicate, missing, extra, malformed, or range-invalid upstream data
- Hidden discovery while disabled, stable non-sensitive failures, and aggregate allowed/denied audit counters
- Native ARM64 official-Termux execution with fixed-path, strict-normalization, overflow, pipe-holder, and client-cancellation cleanup checks in CI

Denied:

- Contacts, SMS, notifications, accounts, location, camera, microphone, accessibility state, installed package inventory, persistent device IDs, and user secrets
- Shell fallback
- Any mutation, including volume setting, or device-control action
- Caller-selected commands, arguments, executable paths, environment, timeouts, or output limits

Required before any future expansion:

- Updated written allowlist and denylist
- Tests proving denied fields are absent
- Runtime status metadata distinguishing read-only status from Android control
- No new dependency unless Security passes exact-head audit
- Exact-head native ARM64 validation of every separately built Android posture

## Gate 3: project-owned service state

Status: implemented for read-only allowlisted project-owned logical service status.

Current scope:

- Status of explicitly allowlisted project-owned services
- Structured service health fields
- No global process listing
- No arbitrary PID or service inspection
- No service mutation or control
- Aggregate audit counter coverage for allowed and denied service-status decisions

Denied:

- Global process listing
- Arbitrary PID inspection
- Command execution
- Reading unrelated process command lines or environment
- Service start, stop, restart, reload, enable, disable, or supervision changes

Required before any future expansion:

- Service allowlist update
- Tests proving unrelated services/processes are not exposed
- Structured unsupported-service errors
- Updated audit-counter or audit-log documentation matching the chosen visibility model

## Gate 4: command execution

Status: the first fixed-profile, read-only diagnostic slice is implemented behind independent compile-time and runtime gates. Arbitrary command execution remains disabled.

The detailed gate design is maintained in [`command-execution-gate.md`](command-execution-gate.md).

Implemented scope:

- Separate `command-execution` feature, including `mcp-runtime`
- Separate `MCP__COMMAND__ENABLED=true` runtime opt-in, defaulting to disabled
- `run_command_profile` with a one-property closed schema and exact profile enum
- Exact current server executable only; no `PATH` lookup
- Fixed complete argv for `server_version`, `server_help`, and `execution_boundary`
- First canonical configured safe root as cwd, empty environment, and null stdin
- Five-second deadlines, profile-specific stdout/stderr ceilings, and two non-queueing concurrency permits
- The cancellation-safe shared process supervisor with process-group isolation, immediate termination, cleanup reserve, and authoritative direct-child reaping
- UTF-8 and zero-exit success requirements; stable non-sensitive failures with no partial output
- Hidden disabled discovery, runtime-disabled direct-call denial, and aggregate audit counters using only reason codes and numeric profile ordinals
- Exact-source fifth Android artifact and native ARM64 official-Termux validation of the compile/runtime truth table and fixed boundary

Denied:

- Raw command strings, shells, interpreters, caller-selected programs, argv, paths, environment, stdin, timeouts, or limits
- Profiles with placeholders, credentials, broad host inspection, filesystem mutation, Android control, service/package/process/network mutation, or other side effects
- Raw output or caller values in audit counters

Required before any future expansion:

- Apply the full rejection checklist in [`command-profile-validation.md`](command-profile-validation.md)
- Keep each new capability in a separately reviewed profile or higher-risk gate
- Preserve deterministic native evidence and exact-head CI/Security/Android success
- Never redefine fixed diagnostics as arbitrary or high-impact execution

## Gate 5: high-impact controls

Status: threat model complete; high-impact controls remain disabled.

Examples:

- Package installation or removal
- Service restart or stop
- File deletion outside the staged safe-root write policy
- Network or device configuration changes
- Any Android device-control action

The detailed threat model is maintained in [`high-impact-controls-threat-model.md`](high-impact-controls-threat-model.md). Future capability-token evaluation must also satisfy [`capability-token-evaluation-contract.md`](capability-token-evaluation-contract.md) before any high-impact runtime gate is wired.

Required before implementation:

- Dedicated threat model
- Explicit capability token or confirmation design
- Dry-run or preview mode where possible
- Full audit trail or explicitly bounded aggregate audit-counter model, with sensitive-data exclusions documented before runtime wiring
- Rollback plan where feasible
- Security review before merge

## Cross-cutting audit coverage

Current staged audit visibility is documented in [`runtime-audit-counters.md`](runtime-audit-counters.md). Filesystem-specific counter expectations are documented in [`filesystem-audit-counter-contract.md`](filesystem-audit-counter-contract.md). The current counter model is deliberately aggregate, in-memory, backend-neutral, and non-retained.

Before any mutating or command-capable gate expands further, add or update audit coverage that records or counts only stable, non-sensitive decision metadata:

- Tool name
- Gate name
- Dry-run, preview, or mutating mode
- Allowed or denied decision
- Non-sensitive reason code
- Size or limit metadata where relevant

Audit counters and any future retained audit logs must not include credential material, raw file contents, raw filesystem paths, environment values, runtime output, unfixed command text, Android identifiers, hostnames, usernames, global process inventories, bearer material, or arbitrary caller-supplied strings.

Originally added for #138; synchronized to current project governance by #165.
