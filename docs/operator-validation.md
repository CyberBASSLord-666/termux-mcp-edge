# Operator runtime validation checklist

This checklist gives advanced Termux MCP Edge operators a repeatable way to validate the stable MCP transport and staged tool authority without expanding the MCP surface.

Use it after a local build, configuration change, release candidate, or manual dispatch/tag build when you need evidence that the runtime still matches the staged capability model.

## Validation posture

The expected posture is narrow and fail-closed:

- In static-token mode, the complete `/mcp` route requires the configured bearer token before transport validation, JSON-RPC parsing, discovery, or invocation.
- Explicit unauthenticated development mode is accepted only when startup validates a loopback bind.
- `runtime_status`, `platform_info`, `android_status`, `project_service_status`, `create_directory`, `copy_file`, `find_paths`, `hash_file`, `list_directory`, `path_metadata`, `read_binary_file`, `read_binary_range`, `read_file`, `read_text_range`, `search_text`, and `write_file` are the baseline tools expected in authenticated discovery. `android_battery_status`, `android_volume_status`, `set_android_volume`, and `run_command_profile` are expected only when their respective compile-time and runtime gates are enabled.
- `create_directory`, `copy_file`, and `write_file` remain dry-run by default. Directory mutation additionally requires its default-disabled runtime gate and one target-bound request grant; `dry_run:false` alone must fail. Copy and write retain their existing explicit posture and safe-root controls.
- Filesystem creation, reads, listings, searches, and writes remain bounded to configured safe roots.
- `project_service_status` remains limited to explicitly allowlisted project-owned logical services.
- Android status remains read-only allowlisted metadata, not Android platform control.
- Shell access, arbitrary command execution, global process inventory, service mutation, package management, network mutation, and device controls beyond exact request-authorized volume remain unavailable.

## Preflight

Before validating behavior, confirm the operator configuration is deliberately narrow:

1. Build with the intended feature set: normally `--features mcp-runtime`, `--features android-battery-status` for battery validation, `--features android-volume-status` for volume-status validation, `--features android-volume-control` for volume-control validation, or `--features command-execution` for fixed-command validation.
2. Use a strong static bearer token for any deployment that is not explicitly loopback-development only.
3. Protect `$HOME/.config/termux-mcp-edge/runtime.env` with mode `0600`; do not echo the token or use shell tracing while it is loaded.
4. Use localhost-only unauthenticated mode only when the server is bound to a loopback address and not exposed through a tunnel, LAN listener, or reverse proxy.
5. Keep `MCP__TRANSPORT__ALLOWED_HOSTS` and `MCP__TRANSPORT__ALLOWED_ORIGINS` exact and minimal.
6. Leave `MCP__TRANSPORT__SSE_ENABLED=false` unless finite response replay is required; enabling it does not permit broadcast or long-lived server queues.
7. Keep filesystem safe roots limited to a dedicated project directory, not broad shared storage such as `/storage/emulated/0` or `/sdcard`.
8. Leave `MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=false` and `MCP__ANDROID__VOLUME_CONTROL_ENABLED=false` unless their exact mutations are operationally required. Either enabled gate requires static-token auth and a paired lowercase `MCP__CAPABILITY__KEY_ID` plus 64-lowercase-hex `MCP__CAPABILITY__HMAC_KEY_HEX`; keep both secrets owner-readable and out of transcripts.

## Authentication checks

For static-token validation, load the protected token into a temporary shell variable without printing it:

```bash
MCP_TEST_TOKEN="$(sed -n 's/^MCP__AUTH__STATIC_TOKEN=//p' "$HOME/.config/termux-mcp-edge/runtime.env")"
```

Prove all of the following:

- A `/mcp` request with no `Authorization` header receives HTTP 401.
- The response includes `WWW-Authenticate: Bearer` and `Cache-Control: no-store`.
- Missing, malformed, oversized, and incorrect credentials produce the same non-sensitive `unauthorized` response shape.
- The response never includes the configured or presented token.
- A correct `Authorization: Bearer ${MCP_TEST_TOKEN}` header reaches transport validation and MCP handling.
- Authentication rejection happens before invalid Host/Origin or malformed JSON is processed.
- `/health` and `/ready` remain available without credentials and return only coarse non-secret operational status.

## Protocol and session checks

Use the initialization sequence in [`VALIDATION.md`](VALIDATION.md) and prove all of the following:

- POST requires `Content-Type: application/json` and explicit `Accept: application/json, text/event-stream`.
- A schema-valid initialize request negotiates `2025-11-25` and returns one UUID `MCP-Session-Id`; invalid initialize params allocate no session.
- Subsequent requests require the returned session ID and `MCP-Protocol-Version: 2025-11-25` in addition to normal bearer authentication.
- Ping works while the session is pending, but discovery and invocation remain blocked until `notifications/initialized` receives HTTP 202 with no body.
- Separate sessions do not share pending/active state.
- With SSE disabled, a valid GET with `Accept: text/event-stream` returns the documented HTTP 405 and creates no replay state.
- With SSE enabled in a controlled run, eligible POST responses contain one empty primer and one terminal response; GET plus the exact primer `Last-Event-ID` replays only the terminal event, while malformed, evicted, and cross-session cursors fail closed.
- DELETE returns HTTP 204, and later use of that identifier returns HTTP 404.
- Missing lifecycle headers fail with HTTP 400; unknown, expired, terminated, malformed, or duplicate session headers fail without reflecting the presented value.
- A process restart clears in-memory sessions; clients reconnect by sending initialize without a prior session header.

Clear the temporary variable after validation:

```bash
unset MCP_TEST_TOKEN
```

## Discovery checks

A valid runtime discovery pass proves presence and absence:

- An unauthenticated caller receives no tool list in static-token mode.
- An authenticated `tools/list` call includes the sixteen baseline tools. Battery, volume-status, volume-control, and fixed-command tools are absent by default; each appears only in its explicitly enabled posture. An all-feature test build has twenty tools only when all four runtime flags are enabled.
- `tools/list` does not include arbitrary command execution, shells, broader Android control, process listing, service mutation, package management, arbitrary network mutation, environment inspection, or token-management tools.
- Tool descriptions and schemas continue to communicate safe-root, read-only, dry-run, and allowlist boundaries where applicable.
- With the directory mutation gate disabled, the `create_directory` schema constrains `dry_run` to `true`. With it enabled, that constraint is absent and the description names the header grant requirement; neither posture exposes an issuer tool.

Discovery is not sufficient by itself. A tool being absent from discovery is the first guardrail, but each boundary below should also be checked through representative authenticated calls.

## Runtime status and audit-counter checks

Call `runtime_status` before and after representative allowed and denied authenticated tool calls.

Expected evidence:

- `structuredContent.auditCounters` is present when the audit snapshot is available.
- Allowed and denied totals move only in response to staged tool-gate decisions.
- Authentication failures do not enter MCP tool dispatch or expose tool audit data.
- `by_tool` uses stable staged tool names.
- `by_reason_code` uses stable low-cardinality reason codes.
- Counters do not include raw paths, file contents, command output, command arguments, environment values, hostnames, usernames, Android identifiers, private device metadata, bearer values, raw capability tokens, or arbitrary caller-provided strings.
- Restarting the process resets the in-memory counters.
- `createDirectoryMutationEnabled`, `createDirectoryMutationMode`, `createDirectoryGrantRequired`, `createDirectoryGrantHeader`, and `createDirectoryGrantTtlSeconds` accurately report only the public posture and never key, token, target, session, or replay state.
- `androidVolumeControlCompiled`, `androidVolumeControlEnabled`, `androidVolumeControlMode`, `androidVolumeGrantRequired`, `androidVolumeGrantHeader`, and `androidVolumeGrantTtlSeconds` report the same bounded public truth without private grant state.

Audit counters are evidence of gate decisions, not an authorization mechanism and not a retained activity log. The authoritative counter contract is maintained in [`runtime-audit-counters.md`](runtime-audit-counters.md).

## Filesystem checks

Use a dedicated safe-root test directory. Validate all of the following with authenticated calls in static-token mode:

- Listing a safe-rooted directory succeeds with a `safe_root_listing`-style allowed decision.
- `create_directory` with omitted `dry_run` or `dry_run:true` validates one absent target without mutation. With the gate disabled, explicit mutation returns HTTP 403. With it enabled, prove that missing, malformed, wrong-context, other-session, other-principal, other-root, other-target, expired, future-issued, unknown-version/key, invalid-signature, and replayed grants all fail closed without mutation or reflection.
- Use the exact candidate's local `--issue-create-directory-grant` flow for one absent target. Send that grant on a dry run and then on `dry_run:false` to prove preview does not consume it; verify mode `0700`; remove the created empty test directory and replay the grant to prove HTTP 403 `capability_grant_replayed`. Run the concurrent replay regression or canonical validator to prove at most one mutation attempt. See [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md).
- `copy_file` with omitted `dry_run` or `dry_run:true` validates one regular source and absent destination without mutation; explicit `dry_run:false` copies exact binary bytes with mode `0600`. Prove the 1 MiB boundary, same/existing/missing/directory/link/outside denials, cross-root operation, response preflight before mutation, no content reflection, and stable private audit reasons; see [`SAFE_ROOT_FILE_COPY.md`](SAFE_ROOT_FILE_COPY.md).
- `hash_file` returns the exact lowercase SHA-256 digest and bytes hashed for one regular file without returning its path or content. Prove binary and empty inputs, the exact 16 MiB boundary, one-byte-over/missing/outside/link/non-regular rejection, full-response preflight, descriptor exchange resistance, and digest/path/content-private audit counters; see [`SAFE_ROOT_FILE_HASHING.md`](SAFE_ROOT_FILE_HASHING.md).
- `read_binary_file` returns canonical padded RFC 4648 base64 and the exact raw byte count for one regular file without returning its path or host metadata. Prove arbitrary binary and empty inputs, the exact 1 MiB boundary, one-byte-over/missing/outside/final-link/linked-parent/non-regular rejection, max-plus-one runtime growth enforcement, full-response preflight before file access, descriptor exchange resistance, and path/content-private audit counters; see [`SAFE_ROOT_BINARY_READS.md`](SAFE_ROOT_BINARY_READS.md).
- `read_binary_range` returns canonical padded RFC 4648 base64, exact offset/returned/file sizes, and explicit EOF without returning path or host metadata. Prove arbitrary slices and EOF, the exact 256 KiB range and 64 MiB file boundaries, one-byte-over/offset-past-EOF/missing/outside/final-link/linked-parent/non-regular rejection, concurrent size-change rejection, full-response preflight, descriptor exchange resistance, and path/content-private audit counters; see [`SAFE_ROOT_BINARY_RANGES.md`](SAFE_ROOT_BINARY_RANGES.md).
- `read_text_range` returns only complete UTF-8 code points, exact current/next/file byte offsets, and explicit EOF without returning path or host metadata. Prove multi-byte pagination, boundary deferral, exact 256 KiB range and 64 MiB file ceilings, midpoint/offset-past-EOF/invalid-or-truncated-UTF-8/missing/outside/link/non-regular rejection, concurrent size-change rejection, worst-case JSON-escape response preflight, descriptor exchange resistance, and path/content-private audit counters; see [`SAFE_ROOT_TEXT_RANGES.md`](SAFE_ROOT_TEXT_RANGES.md).
- `find_paths` returns only ordered path/kind matches and bounded counters for one literal basename substring. Prove every kind filter, default/exact depth, empty and deterministic results, 8,192-entry/512-match/262,144-byte boundaries, outside/linked-parent/final-link/special/invalid-UTF-8 handling, oversized-ID preflight before argument/filesystem work, and query/content-private audit counters; see [`SAFE_ROOT_PATH_DISCOVERY.md`](SAFE_ROOT_PATH_DISCOVERY.md).
- `path_metadata` returns only normalized path, `regular_file`/`directory` kind, nullable file size and RFC 3339 modification time, and `maxResponseBytes:16384`; links, unsupported types, content, inode/device/UID/GID/mode/access-time fields, and oversized envelopes fail closed.
- Reading a bounded UTF-8 file under a safe root succeeds with a `safe_root_read`-style allowed decision.
- Literal `search_text` finds path/line/byte-column locations without returning matching content or echoing the query; depth, query, file, aggregate-byte, match, and response limits remain fixed.
- Search skips symlinks, non-regular files, oversized files, and invalid UTF-8 without escaping the safe root or reflecting raw operating-system errors.
- Reading or listing a path outside the configured safe root is denied with a stable outside-safe-root reason code.
- Excessive read or write sizes are denied with stable byte-limit reason codes.
- `write_file` with omitted `dry_run` or `dry_run:true` returns a preview and does not mutate the file.
- `write_file` with `dry_run:false` mutates only a safe-rooted target and is still bounded by size and path validation.
- Symlink escapes remain denied.

Filesystem counter expectations are maintained in [`filesystem-audit-counter-contract.md`](filesystem-audit-counter-contract.md).

## Project service status checks

Use the documented project-owned service name first.

Expected evidence:

- `project_service_status` succeeds for an explicitly allowlisted project-owned logical service such as `mcp_runtime`.
- Missing, malformed, or unsupported service names fail with structured errors and stable reason codes.
- The tool does not expose arbitrary service discovery, global process lists, PIDs, command lines, environment values, service control, or supervision mutation.

## Android status checks

Expected evidence:

- `android_status` returns only read-only allowlisted Android/Termux status metadata.
- It does not expose contacts, SMS, notifications, accounts, location, camera, microphone, accessibility state, installed package inventory, persistent device identifiers, user secrets, shell fallback, or device-control actions.
- Read-only Android status must not be treated as completion of the Android platform-control gate.

## Optional Android battery checks

Use the dedicated procedure in [`ANDROID_BATTERY_STATUS.md`](ANDROID_BATTERY_STATUS.md). Prove both gate states rather than leaving the runtime enabled after one successful call.

Expected disabled evidence:

- A normal `mcp-runtime` build reports `androidBatteryStatusCompiled:false`, hides the tool, and returns `battery_feature_not_compiled` for a direct invocation.
- An `android-battery-status` build with the runtime flag absent or `false` reports compiled but disabled, hides the tool, and returns `battery_runtime_disabled` for a direct invocation.
- In both cases `androidDeviceControl`, command execution, and high-impact tools remain false.

Expected enabled evidence:

- Startup uses the `android-battery-status` build and `MCP__ANDROID__BATTERY_STATUS_ENABLED=true`.
- `tools/list` advertises the closed empty-object schema for `android_battery_status`.
- The fixed Termux:API executable exists and a call completes within five seconds.
- The response contains only documented normalized fields and units; it contains no `technology`, vendor string, identifier, path, environment value, raw output, or stderr.
- Non-empty arguments return JSON-RPC `-32602`.
- Provider failures use stable `battery_*` reason codes and do not reveal process details.
- Endless stdout/stderr, a descendant retaining either pipe, and a disconnected caller all terminate within the same fixed deadline, leave no provider process-group survivor, and do not accumulate background supervisors.
- Successful and denied calls increment only the documented aggregate audit labels.

The native ARM64 official-Termux CI gate performs these automated process/transport checks with a fixed-path fixture and publishes strict v2 battery evidence. A physical release check, when required by the observation classifier, is for battery/OEM/Android behavior only; routine feature PRs do not require an operator to repeat a 60-minute idle window.

## Optional Android volume checks

Use [`ANDROID_VOLUME_STATUS.md`](ANDROID_VOLUME_STATUS.md) and prove both gate states.

Expected disabled evidence:

- A normal `mcp-runtime` build reports `androidVolumeStatusCompiled:false`, hides the tool, and returns `volume_feature_not_compiled` for direct invocation.
- An `android-volume-status` build with its runtime flag absent or `false` reports compiled but disabled, hides the tool, and returns `volume_runtime_disabled`.
- `androidDeviceControl`, command execution, and high-impact tools remain false.

Expected enabled evidence:

- Startup uses the `android-volume-status` build and `MCP__ANDROID__VOLUME_STATUS_ENABLED=true`.
- `tools/list` advertises a closed empty-object schema for `android_volume_status`.
- A call invokes only the fixed `termux-volume` zero-argument status mode and completes within five seconds.
- `structuredContent.streams` contains exactly `alarm`, `call`, `music`, `notification`, `ring`, and `system` in that order, with integer `volume` and `maxVolume` values in range.
- Extra, unknown, duplicate, missing, non-integer, or range-invalid provider data fails with a stable `volume_*` reason and is not reflected.
- Non-empty caller arguments return JSON-RPC `-32602` and cannot select the upstream command's volume-setting mode.
- Output overflow, pipe-holding descendants, and disconnected callers leave no provider process-group survivor or detached supervisor.
- Successful and denied calls increment only documented stable aggregate audit labels.

The native ARM64 official-Termux workflow automates these checks and publishes strict v1 volume evidence. Physical-device audio-policy or OEM behavior remains separate release evidence when applicable; routine feature development does not require a long idle observation.

## Optional Android volume-control checks

Use [`ANDROID_VOLUME_CONTROL.md`](ANDROID_VOLUME_CONTROL.md) and prove the complete compile/runtime/auth/key/request truth table.

- An incompatible binary rejects an enabled control flag before listening; a control binary with the flag absent hides the tool and reports disabled.
- Enabled discovery exposes only the exact six-stream/integer/optional-boolean closed schema and never an issuer tool.
- Preview performs a fresh status read, enforces the live maximum, invokes no setter, and does not consume a matching grant.
- Missing, malformed, wrong-context, other-principal/session/capability/stream/level, expired, future-issued, unknown-version/key, bad-signature, clock-rollback, state-capacity, and replay grants fail closed without mutation or private reflection.
- The exact binary issues one bound grant from a private literal config; one matching mutation consumes it, uses only fixed `termux-volume stream level`, verifies status, and rejects replay.
- Conflicting mutations fail without queueing. Timeout, output overflow, pipe holders, and caller cancellation leave no process-group survivor; cancellation after consumption does not cancel verification/restoration.
- Forced setter and verification failures prove both rollback-confirmed and rollback-unconfirmed stable reasons. Audit counters retain only stable aggregate labels.

Native ARM64 official-Termux evidence is deterministic development qualification. A later physical check, if the release classifier requires it, evaluates device/OEM audio-policy behavior without repeating an arbitrary idle-monitoring window.

## Optional fixed command profile checks

Use [`command-execution-gate.md`](command-execution-gate.md) and prove the complete compile/runtime truth table.

Expected disabled evidence:

- A default binary rejects `MCP__COMMAND__ENABLED=true` during startup with the stable feature requirement and never opens a listener.
- A `command-execution` binary with the flag absent or false reports `commandExecutionCompiled:true`, `commandExecution:false`, hides `run_command_profile`, and denies direct calls with `command_runtime_disabled` without spawning.
- `arbitraryCommandExecution`, `androidDeviceControl`, and `highImpactTools` remain false.

Expected enabled evidence:

- Startup uses the `command-execution` build and `MCP__COMMAND__ENABLED=true`.
- Discovery advertises only the closed `profile` enum `server_version`, `server_help`, and `execution_boundary`.
- Version and help return bounded output from the exact server executable.
- Boundary self-check returns exactly `termux-mcp-command-boundary ok`, proving empty environment, null stdin, and non-root safe-root cwd without reflecting their values.
- Missing/unknown profiles and every attempted `command`, `program`, `argv`, `workingDirectory`, `environment`, `stdin`, `timeout`, `stdoutLimit`, or `stderrLimit` field fail before spawn.
- Runtime status reports `fixed_read_only_server_diagnostics` while arbitrary execution and high-impact controls remain false.
- Audit counters record three allowed fixed profiles and stable denied reasons without profile text, argv, cwd, environment, or output.

The native ARM64 official-Termux command gate automates these deterministic checks and publishes strict v1 evidence. It does not run or require a long monitoring window.

## Capability-token boundary checks

Capability-token primitives are currently inert policy scaffolding for future high-impact gates. They are separate from the static bearer token used to authenticate the MCP transport.

Expected evidence:

- No raw high-impact capability-token issuance, persistence, bearer parsing, validation, or serialization is exposed by the runtime.
- No high-impact MCP tool is enabled by the presence of capability-token primitives.
- Future capability-token evaluation must remain exact-match, fail-closed, bounded to non-secret metadata, and audited only with stable non-sensitive labels.

The capability-token evaluation contract is maintained in [`capability-token-evaluation-contract.md`](capability-token-evaluation-contract.md).

## Failure interpretation

Treat any of the following as a blocker for a staged runtime PR or release candidate:

- Static-token mode permits unauthenticated `/mcp` discovery or invocation.
- Authentication failures reveal token values or reach JSON-RPC/tool dispatch.
- Initialization, media negotiation, protocol headers, or per-session lifecycle gating can be bypassed.
- A notification/client response receives a JSON-RPC response body, or a batch array is accepted.
- Discovery exposes a tool outside the staged baseline.
- A read-only metadata tool exposes private identifiers, secrets, environment values, filesystem paths outside filesystem tools, process inventory, or command output.
- The battery tool is discovered without both opt-ins, accepts caller-selected process inputs, exceeds its fixed time/output ceilings, or reflects a dropped upstream field.
- The volume-status tool is discovered without both opt-ins, accepts any argument, reaches volume mutation, returns a non-canonical/partial stream set, exceeds its fixed bounds, or reflects unrecognized upstream data.
- The volume-control tool is discovered without all gates, accepts a non-closed input, mutates without an exact single-use grant, queues conflicts, skips fresh bounds/verification/recovery, exposes caller process controls, or reflects private/provider material.
- The command tool is discovered without both opt-ins, accepts any caller-controlled process field, invokes a non-current executable, inherits environment or stdin, escapes its safe-root cwd, exceeds its fixed bounds, reflects failed output, or enables arbitrary/high-impact execution.
- Filesystem tools can escape configured safe roots; any mutation occurs without its explicit posture; or `create_directory` mutation occurs without the enabled gate and exact single-use request grant.
- Audit counters serialize raw caller values or high-cardinality private metadata.
- General capability-token primitives become a live high-impact authorization surface without a separate focused gate.
- Any executable authority beyond the fixed diagnostic profiles and exact-stream volume control, or any service/package/network/other high-impact action, appears without its own documented opt-in gate, tests, and audit contract.

When a blocker is found, keep remediation narrow: preserve existing response contracts unless the fix explicitly documents an additive change, and do not combine runtime behavior changes with dependency or workflow maintenance.
