# Android Volume Status Tool

## Purpose and authority boundary

`android_volume_status` is an authenticated, read-only MCP tool for bounded Android audio-stream volume telemetry from Termux:API. It is available only when all three gates are satisfied:

1. the binary is built with `--features android-volume-status`;
2. `MCP__ANDROID__VOLUME_STATUS_ENABLED=true` passes startup validation;
3. the existing MCP authentication, request-limit, Host, Origin, initialization, protocol-version, and session checks pass.

The feature includes `mcp-runtime`; it does not enable volume mutation, Android control, arbitrary commands, shell interpolation, `rish`, Shizuku, package/service/network mutation, or any high-impact tool. The upstream `termux-volume` command has a separate argument-taking mutation mode, but this provider is structurally fixed to the command's zero-argument status mode. Callers cannot select a stream, volume, executable, argument, or process setting.

The runtime flag defaults to `false`. When disabled, the tool is absent from `tools/list`. A direct call returns a stable, non-sensitive tool error without process details.

## Device prerequisites

Install and configure the official Termux:API add-on and its Termux package so this fixed executable exists:

```text
/data/data/com.termux/files/usr/bin/termux-volume
```

The server executes that exact absolute path directly. It supplies no arguments, attaches stdin to `/dev/null`, clears the inherited environment, fixes the child working directory to `/`, and neither invokes a shell nor searches `PATH`. The upstream executable may itself be a Termux-provided wrapper; the server never constructs shell text or accepts a caller-selected command.

Build and enable the posture explicitly:

```bash
cargo build --release --features android-volume-status
export MCP__ANDROID__VOLUME_STATUS_ENABLED=true
```

The normal static-token and transport configuration remains required. A binary without the compile-time feature rejects `MCP__ANDROID__VOLUME_STATUS_ENABLED=true` during startup.

The provider contract is pinned to the official zero-argument wrapper and six-stream Android API response:

- [`termux-volume.in`](https://github.com/termux/termux-api-package/blob/0e3f9222eea7760c76ea6368dadbdf884ab85fbf/scripts/termux-volume.in)
- [`VolumeAPI.java`](https://github.com/termux/termux-api/blob/760c1777950d69d87b20c0147588b7b660f29135/app/src/main/java/com/termux/api/apis/VolumeAPI.java)

## MCP contract

The advertised input schema is closed and accepts only omitted arguments or an empty object:

```json
{
  "type": "object",
  "properties": {},
  "additionalProperties": false
}
```

Successful `structuredContent` has one exact public shape:

```json
{
  "streams": [
    { "stream": "alarm", "volume": 4, "maxVolume": 7 },
    { "stream": "call", "volume": 1, "maxVolume": 5 },
    { "stream": "music", "volume": 5, "maxVolume": 15 },
    { "stream": "notification", "volume": 3, "maxVolume": 7 },
    { "stream": "ring", "volume": 6, "maxVolume": 7 },
    { "stream": "system", "volume": 2, "maxVolume": 7 }
  ]
}
```

The response must contain each of `alarm`, `call`, `music`, `notification`, `ring`, and `system` exactly once. The server always returns them in that canonical order regardless of upstream order. Every entry must contain exactly `stream`, integer `volume`, and integer `max_volume`; unknown, duplicate, missing, or extra fields fail closed. `max_volume` must be `1` through `10000`, and `volume` must be `0` through that stream's maximum. The public response renames `max_volume` to `maxVolume`.

Raw output, stderr, executable paths, environment values, Android identifiers, and unrecognized vendor fields are never reflected.

## Process and resource limits

Each invocation has fixed ceilings:

| Resource | Limit |
|---|---:|
| Normal wall-clock budget | 5 seconds, including a reserved cleanup window |
| Standard output | 8 KiB |
| Standard error | 4 KiB |
| Arguments | 0 |
| Stdin | Null |
| Inherited environment | Cleared |
| Working directory | Fixed `/` |

Battery and volume providers share one cancellation-safe supervisor implementation. It observes the direct child, both output streams, caller cancellation, and one absolute normal-operation budget; isolates the provider in its own process group; terminates the group on success, overflow, timeout, cancellation, or failure; closes pipes; and synchronously reaps the direct child. If confirmed reaping exhausts the cleanup reserve, the stable wait failure overrides the primary result and the independently owned supervisor remains responsible until collection. Simultaneous terminal events use stable precedence: cancellation, deadline, stdout, stderr, then child completion. No reader task or unbounded pipe join can outlive the invocation.

Output is parsed only after a successful exit and complete bounded reads.

## Stable failure contract

Operational failures return HTTP 200 with `isError: true`, `structuredContent.error: "android_volume_status_unavailable"`, and one stable `reasonCode`:

- `volume_feature_not_compiled`
- `volume_runtime_disabled`
- `volume_api_unavailable`
- `volume_api_spawn_failed`
- `volume_api_wait_failed`
- `volume_api_timeout`
- `volume_stdout_limit_exceeded`
- `volume_stderr_limit_exceeded`
- `volume_api_failed`
- `volume_output_invalid_utf8`
- `volume_output_invalid_json`
- `volume_output_invalid_field`

Non-empty or non-object arguments use JSON-RPC `-32602` with `arguments_not_empty_or_not_object`. No failure includes the program path, raw API response, stderr, exit code, token, session identifier, or caller data.

## Audit behavior

In-memory aggregate audit counters retain only the stable tool name, gate name, allowed/denied decision, and reason code. Successful reads use `volume_status_read`. Disabled gates, invalid arguments, and provider failures use the reason codes above. Counters reset at process restart and are neither authorization nor retained activity logs.

## Validation

Repository tests cover the exact six-stream contract, canonical ordering, field names and integer ranges, duplicate/missing/unknown/extra rejection, invalid JSON and UTF-8, output ceilings, timeout and provider failure, fixed zero-argument invocation, environment clearing, compile/runtime truth tables, discovery, direct invocation, error shape, audit counts, authoritative late reaping, and caller-cancellation cleanup through the shared supervisor.

The Android workflow builds a fourth independent `android-volume-status` AArch64 artifact and executes it in the pinned official Termux image on a native ARM64 runner. Its temporary fixed-path fixture proves exact artifact provenance, enabled and disabled discovery, zero arguments, fixed working directory, environment clearing, strict normalization, canonical ordering, unrecognized-field rejection without reflection, prompt stdout/stderr overflow handling, process-group and pipe-holder cleanup, caller-cancellation cleanup, stable provider failure, and continued absence of device control, command execution, and high-impact tools. The sanitized report conforms to [`android-volume-emulated-evidence-schema-v1.json`](android-volume-emulated-evidence-schema-v1.json) and is automated development evidence, not a claim about physical-device audio policy or OEM behavior.

Volume mutation, audio routing, media control, microphone access, sensors, location, contacts, SMS, notifications, UI/accessibility automation, logcat, camera, package/service/network mutation, `rish`, and arbitrary command execution are outside this tool's scope.
