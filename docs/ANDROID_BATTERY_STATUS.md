# Android Battery Status Tool

## Purpose and authority boundary

`android_battery_status` is an authenticated, read-only MCP tool for bounded battery and battery-temperature telemetry from Termux:API. It is available only when all three gates are satisfied:

1. the binary is built with `--features android-battery-status`;
2. `MCP__ANDROID__BATTERY_STATUS_ENABLED=true` passes startup validation;
3. the existing MCP transport authentication, request-limit, Host, Origin, initialization, protocol-version, and session checks pass.

The feature includes `mcp-runtime`; it does not enable Android control, arbitrary commands, shell interpolation, `rish`, Shizuku, package/service/network mutation, or any high-impact tool. The runtime flag defaults to `false`. When disabled, the tool is absent from `tools/list`. A direct call still fails with a stable, non-sensitive tool error so clients do not receive internal process details.

## Device prerequisites

The device operator must install and configure the official Termux:API add-on and its Termux package so this fixed executable exists:

```text
/data/data/com.termux/files/usr/bin/termux-battery-status
```

The server executes that exact absolute path directly. It supplies no arguments or stdin, clears the inherited environment, fixes the child working directory to `/`, and does not invoke a shell or search `PATH`. The upstream executable may itself be a Termux-provided wrapper; the server never constructs shell text or accepts a caller-selected command.

Build and enable the posture explicitly:

```bash
cargo build --release --features android-battery-status
export MCP__ANDROID__BATTERY_STATUS_ENABLED=true
```

The normal static-token and transport configuration remains required. A binary without the compile-time feature rejects `MCP__ANDROID__BATTERY_STATUS_ENABLED=true` during startup.

## MCP contract

The advertised input schema is closed and accepts only omitted arguments or an empty object:

```json
{
  "type": "object",
  "properties": {},
  "additionalProperties": false
}
```

The response contains only recognized fields that were present and valid in the upstream response:

| Field | Type | Unit or values |
|---|---|---|
| `present` | boolean | Battery-present state |
| `health` | string | Bounded uppercase Termux:API label |
| `plugged` | string | Bounded uppercase Termux:API label |
| `status` | string | Bounded uppercase Termux:API label |
| `temperature_celsius` | number | Degrees Celsius |
| `voltage_millivolts` | integer | Millivolts |
| `current_microamps` | integer | Microamperes |
| `current_average_microamps` | integer | Microamperes |
| `percentage` | integer | `0` through `100` |
| `level` | integer | Raw level; never greater than `scale` when both exist |
| `scale` | integer | Raw positive scale |
| `charge_counter_microamp_hours` | integer | Microampere-hours |
| `energy_nanowatt_hours` | integer | Nanowatt-hours |
| `cycle_count` | integer | Non-negative cycle count |

Unknown upstream fields are dropped. In particular, `technology`, persistent identifiers, vendor extensions, raw output, stderr, filesystem paths, and environment values are never reflected. Known fields with an invalid type, range, label, or cross-field relationship fail closed instead of returning a partially ambiguous response.

Battery current sign is device-reported. Some devices report a sign that does not align with charging state; the server does not reinterpret or invert it.

## Process and resource limits

Each invocation has these fixed ceilings:

| Resource | Limit |
|---|---:|
| Wall-clock time | 5 seconds |
| Standard output | 16 KiB |
| Standard error | 4 KiB |
| Arguments | 0 |
| Stdin | Null |
| Inherited environment | Cleared |
| Working directory | Fixed `/` |

One cancellation-safe supervisor concurrently observes the direct child, both output streams, caller cancellation, and a single end-to-end deadline. The provider starts as the leader of an isolated process group. Crossing either byte ceiling terminates the whole group immediately; timeout, wait/read failure, client cancellation, and successful completion also close both pipes, terminate any remaining descendants, and synchronously reap the direct child within the original five-second wall-clock ceiling. A bounded final portion of that ceiling is reserved for cleanup. Simultaneous terminal events use stable precedence: cancellation, deadline, stdout, stderr, then child completion. No reader task or unbounded drain/join can outlive the invocation.

Process output is parsed only after a successful exit and complete bounded reads. It is never included in an error response or audit counter.

## Stable failure contract

Operational failures return an MCP tool result with HTTP 200, `isError: true`, `structuredContent.error: "android_battery_status_unavailable"`, and one stable `reasonCode`:

- `battery_feature_not_compiled`
- `battery_runtime_disabled`
- `battery_api_unavailable`
- `battery_api_spawn_failed`
- `battery_api_wait_failed`
- `battery_api_timeout`
- `battery_stdout_limit_exceeded`
- `battery_stderr_limit_exceeded`
- `battery_api_failed`
- `battery_output_invalid_utf8`
- `battery_output_invalid_json`
- `battery_output_invalid_field`

Non-empty or non-object arguments use the normal JSON-RPC `-32602` invalid-params response and reason code `arguments_not_empty_or_not_object`. No response includes the program path, raw API response, stderr, exit code, token, session identifier, or caller data.

## Audit behavior

The existing in-memory aggregate audit counters record only the stable tool name, gate name, allowed/denied decision, and reason code. Successful reads use `battery_status_read`. Disabled gates, invalid arguments, and provider failures use the reason codes above. Counters reset at process restart and are not retained activity logs.

## Validation

Repository tests cover strict parsing, field redaction, type/range rejection, exact and over-limit output, repeated endless stdout/stderr, deterministic overflow-before-timeout behavior, pipe-holding descendants, process-group termination, caller cancellation, synchronous direct-child reaping, supervisor/task accumulation, timeout, non-zero exit, invalid UTF-8, missing API executable, compile/runtime gate truth tables, discovery, direct invocation, error shape, and audit counts.

The Android workflow builds a separate `android-battery-status` AArch64 artifact and executes it in the pinned official Termux image on a native ARM64 runner. That gate installs a temporary fixed-path API fixture and proves enabled and disabled discovery, fixed-working-directory/no-argument/environment-cleared execution, normalization/redaction, immediate endless-output rejection, stdout/stderr pipe-holder cleanup, client-cancellation cleanup, process-group termination, stable provider failure, and continued absence of device-control, command, and high-impact tools. Its v2 evidence is strict and sanitized. This automated gate does not claim battery, OEM thermal-management, mobile-radio, or Android background-process behavior on physical hardware; release evidence classifies those separately without requiring a long observation for every development PR.

Sensors, location, contacts, SMS, notifications, UI automation, accessibility automation, logcat, camera, microphone, package/service/network mutation, `rish`, and arbitrary command execution are explicitly outside this tool's scope.
