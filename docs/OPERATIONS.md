# Operations Guide

## Purpose

Termux MCP Edge runs as a small Rust/Axum service on Android through Termux. The supported production path uses versioned releases, the fixed `mcp_runtime` runit service, fail-closed bearer authentication, mobile-conscious request limits, exact transport allowlists, and safe-rooted filesystem tools.

## Baseline operating model

- Rust single binary.
- `GET /health` and `GET /ready` operational endpoints.
- Optional authenticated stable MCP 2025-11-25 `POST`, `GET`, and `DELETE /mcp` handling; GET returns 405 because SSE is not offered.
- Authentication before concurrency, timeout, body-size, Host, Origin, parsing, discovery, and dispatch.
- Four concurrent authenticated MCP requests by default.
- Thirty-second request timeout by default.
- Two-MiB request-body ceiling by default.
- Versioned Termux release directories with atomic `current` and `previous` links.
- Fixed `mcp_runtime` runit service only.
- Dedicated safe-root defaults, default-disabled directory mutation, and request-scoped authorization for each directory creation attempt.

## Android hardening

1. Set Termux battery usage to unrestricted.
2. Remove Termux from sleeping and deep-sleeping app lists.
3. Use `termux-wake-lock` only when persistent background operation is required.
4. On Android 14 or later, enable **Developer options → Disable child process restrictions**.
5. Avoid direct public port exposure. Use a reviewed VPN or named-tunnel path only after authentication is configured and tested.
6. Keep the mobile request-limit defaults unless target-device measurements justify a reviewed increase.

For a reviewed named-tunnel deployment, use [`NAMED_TUNNEL_SETUP.md`](NAMED_TUNNEL_SETUP.md). The helper requires explicit tunnel/hostname arguments, supports a zero-call `--dry-run`, requires `--create` before login or creation, and never overwrites an existing DNS record.

## Install and service supervision

Install prerequisites:

```bash
pkg update
pkg install bash coreutils curl file termux-services
```

Use [`TERMUX_DEPLOYMENT.md`](TERMUX_DEPLOYMENT.md) for initial install, upgrade, rollback, recovery, status, and uninstall. New deployments should use `scripts/termux_deploy.sh`; it creates and manages only:

```text
$PREFIX/var/service/mcp_runtime/run
```

The legacy static `scripts/runit/mcp-server/run` file is not the canonical versioned deployment path. Do not run both service definitions simultaneously.

Check service state:

```bash
sv status "$PREFIX/var/service/mcp_runtime"
```

The generated service reads a private `runtime.env` as literal allowlisted `NAME=value` data. It does not evaluate the configuration as shell program text.

## Runtime probes

```bash
curl -fsS http://127.0.0.1:8000/health
curl -fsS http://127.0.0.1:8000/ready | jq
```

Expected health response:

```text
ok
```

When `mcp-runtime` is enabled, readiness reports coarse package, feature, authentication-posture, safe-root-count, and active request-limit metadata. It must not return tokens, raw configuration, private paths, tool discovery, or tool output.

## Authenticated MCP validation

Load the token without printing it:

```bash
MCP_TEST_TOKEN="$(sed -n 's/^MCP__AUTH__STATIC_TOKEN=//p' "$HOME/.config/termux-mcp-edge/runtime.env")"
```

Verify unauthenticated rejection first, then authenticated discovery:

```bash
curl -i -sS \
  -X POST \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  --data '{"jsonrpc":"2.0","id":0,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp

MCP_RESPONSE_HEADERS="$(mktemp)"
curl -sS -D "$MCP_RESPONSE_HEADERS" \
  -X POST \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  --data '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"termux-operations-check","version":"1.0.0"}}}' \
  http://127.0.0.1:8000/mcp | jq -e '.result.protocolVersion == "2025-11-25"'
MCP_SESSION_ID="$(awk 'tolower($1) == "mcp-session-id:" {sub(/^[^:]*:[[:space:]]*/, ""); sub(/\r$/, ""); print; exit}' "$MCP_RESPONSE_HEADERS")"
test -n "$MCP_SESSION_ID"

test "$(curl -sS -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "MCP-Session-Id: ${MCP_SESSION_ID}" \
  --data '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  http://127.0.0.1:8000/mcp)" = 202

curl -sS \
  -H "Authorization: Bearer ${MCP_TEST_TOKEN}" \
  -H 'Host: localhost:8000' \
  -H 'Origin: http://localhost:8000' \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -H 'MCP-Protocol-Version: 2025-11-25' \
  -H "MCP-Session-Id: ${MCP_SESSION_ID}" \
  --data '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  http://127.0.0.1:8000/mcp | jq -e '.result.tools | length == 11'

rm -f "$MCP_RESPONSE_HEADERS"
unset MCP_TEST_TOKEN MCP_SESSION_ID MCP_RESPONSE_HEADERS
```

Do not enable shell tracing, echo token variables, or include credential-bearing commands in screenshots or issue text.

Each process holds at most 64 sessions and expires them after 30 idle minutes. Missing required post-initialize protocol/session headers return HTTP 400; expired, terminated, malformed, or unknown sessions return HTTP 404; capacity exhaustion returns HTTP 503. A client should DELETE a finished session and initialize a new session after HTTP 404 or a server restart. Session IDs do not replace the bearer token.

## Request limits

The listener defaults to `MCP__SERVER__PORT=8000` and accepts only ports `1–65535`. Port `0`, malformed numbers, and present non-Unicode security/network configuration values fail before the listener starts. Only absent variables use defaults.

| Setting | Default | Valid range |
|---|---:|---:|
| `MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS` | `4` | `1–64` |
| `MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS` | `30` | `1–300` |
| `MCP__TRANSPORT__MAX_BODY_BYTES` | `2097152` | `1024–8388608` |

Values outside these ranges fail startup. Increasing concurrency and body size together increases possible peak memory use; evaluate them together on the target device.

Failure semantics:

- HTTP 413 / `mcp_request_body_too_large`.
- HTTP 503 / `mcp_concurrency_limit_reached`, with `Retry-After: 1`.
- HTTP 504 / `mcp_request_timeout`.

Limit failures contain non-sensitive JSON and `Cache-Control: no-store`.

## Current MCP tools

Authenticated discovery currently exposes:

1. `runtime_status` — staged runtime metadata and aggregate non-sensitive audit counters.
2. `platform_info` — non-sensitive platform metadata.
3. `android_status` — read-only allowlisted Android/Termux status metadata.
4. `project_service_status` — read-only allowlisted project service metadata for `mcp_runtime`.
5. `create_directory` — safe-rooted preview by default; one mode-`0700` atomic no-replace mutation only after the dedicated gate and a target-bound single-use grant authorize it.
6. `copy_file` — one binary-safe regular file up to 1 MiB, fixed mode `0600`, atomic no-replace, content-private, dry-run first.
7. `list_directory` — bounded safe-rooted listing.
8. `path_metadata` — bounded safe-rooted regular-file or directory metadata without content or host identifiers.
9. `read_file` — bounded safe-rooted UTF-8 reads.
10. `search_text` — bounded case-sensitive literal UTF-8 location search without content excerpts.
11. `write_file` — safe-rooted, payload-bounded, dry-run-first writes.

An `android-battery-status` binary with `MCP__ANDROID__BATTERY_STATUS_ENABLED=true` additionally exposes `android_battery_status` as the twelfth tool. It is disabled and hidden by default; see [`ANDROID_BATTERY_STATUS.md`](ANDROID_BATTERY_STATUS.md).

An `android-volume-status` binary with `MCP__ANDROID__VOLUME_STATUS_ENABLED=true` instead exposes `android_volume_status` as the twelfth tool. It is independently disabled and hidden by default, uses only the fixed zero-argument `termux-volume` status mode, and never authorizes volume mutation; see [`ANDROID_VOLUME_STATUS.md`](ANDROID_VOLUME_STATUS.md). An all-feature validation build can expose both provider tools when both runtime flags are explicitly enabled.

An `android-volume-control` binary with `MCP__ANDROID__VOLUME_CONTROL_ENABLED=true`, static-token authentication, and the capability key pair exposes `set_android_volume`. It defaults to fresh validated preview. Explicit mutation requires one exact request grant and performs fixed execution, verification, and restoration on failure; see [`ANDROID_VOLUME_CONTROL.md`](ANDROID_VOLUME_CONTROL.md).

A `command-execution` binary with `MCP__COMMAND__ENABLED=true` exposes `run_command_profile` after the eleven baseline tools. It offers only the exact `server_version`, `server_help`, and `execution_boundary` profiles of the current server binary. It remains hidden when disabled; see [`command-execution-gate.md`](command-execution-gate.md). An all-feature validation build exposes fifteen tools only when all four optional runtime flags are explicitly enabled.

The runtime does not expose Android platform control beyond exact request-authorized volume, an arbitrary shell or command runner, global process inventory, arbitrary service inspection, service mutation, package management, network mutation, or unrelated high-impact controls.

Filesystem responses have explicit mobile-oriented ceilings:

- `create_directory` validates one absent child by default. Explicit `dry_run:false` selects mutation but succeeds only when `MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true` and the request carries one unexpired, exact-target, single-use `MCP-Capability-Grant`. Confinement completes before authorization; consumption occurs immediately before the first mutation and survives downstream failure. The operation creates fixed mode `0700`, publishes without replacement, syncs child and parent descriptors, and caps the complete response at 16 KiB; see [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md) and [`SAFE_ROOT_DIRECTORY_CREATION.md`](SAFE_ROOT_DIRECTORY_CREATION.md).
- `copy_file` validates one regular source and absent destination by default and mutates only with explicit `dry_run:false`. It copies at most 1 MiB from the exact held source descriptor, publishes fixed mode `0600` atomically without replacement, verifies identities and sizes, syncs file and parent descriptors, returns no content, and caps the complete response at 16 KiB; see [`SAFE_ROOT_FILE_COPY.md`](SAFE_ROOT_FILE_COPY.md).
- `list_directory` returns at most 4,096 entries and at most 256 KiB for the complete JSON-RPC response. Entries are ordered deterministically by path before publication. `structuredContent.truncated` reports when either ceiling prevented a complete result and the response publishes both limits.
- `path_metadata` returns exactly normalized path, regular-file/directory kind, nullable file size, nullable RFC 3339 modification time, and the fixed 16 KiB full-response ceiling. It does not return content, inode/device/UID/GID/mode/access-time data, link targets, or unsupported object types; see [`SAFE_ROOT_PATH_METADATA.md`](SAFE_ROOT_PATH_METADATA.md).
- `read_file` reads at most 1 MiB of valid UTF-8 and caps the complete JSON-RPC response at 1,114,112 bytes. The file content appears once in `structuredContent.content`; the text content is a fixed-format byte-count summary. JSON escaping that would exceed the response ceiling is rejected with a bounded payload-too-large error.
- `search_text` accepts one literal query of at most 256 UTF-8 bytes, examines at most 8,192 entries and 4,096 files through depth 5, reads at most 1 MiB per file and 8 MiB total, returns at most 256 path/line/byte-column matches, and caps the complete response at 256 KiB. It returns no file-content excerpt or query echo; see [`SAFE_ROOT_TEXT_SEARCH.md`](SAFE_ROOT_TEXT_SEARCH.md).

These response ceilings are independent of the authenticated request-body ceiling and cannot be increased through environment configuration.

## Filesystem safe roots

The default filesystem root is:

```text
/data/data/com.termux/files/home/mcp-files
```

Keep configured roots limited to dedicated project directories. Empty lists, relative roots, filesystem root `/`, traversal, and symlink escapes are rejected. Broad shared Android storage is not a default.

The runtime opens the safe root and walks each descendant with descriptor-relative no-follow operations. Symlinks are not a supported aliasing mechanism inside a safe root. Writes use a private same-directory temporary file, sync the file before atomic rename, and sync the parent directory after rename; an error before the parent sync must be treated as a failed durability confirmation even if the new name is visible.

## Deployment status and recovery

```bash
scripts/termux_deploy.sh status
```

The deployment manager validates `current` and `previous` before reporting them. It rejects links that escape the project releases directory or point to incomplete releases.

For a failed candidate, the manager restores the exact prior link state, removes the candidate, restarts the prior active release, and probes it. For a failed explicit rollback, it restores and re-probes the original active release. Operations are serialized with a project lock and interruption cleanup.

Do not manually repoint release links outside the project releases directory. Preserve persistent configuration during ordinary recovery.

## Release process

1. Run format, workspace/all-target/all-feature Clippy, workspace/all-target/all-feature tests, and deployment shell tests.
2. Build the default, `mcp-runtime`, `android-battery-status`, `android-volume-status`, `android-volume-control`, and `command-execution` release postures.
3. Confirm Security when Cargo, lockfile, or Security-workflow inputs change.
4. Cross-compile and validate all six Android postures, including native ARM64 official-Termux execution and the control/command compile-gate truth tables.
5. Record and verify each posture-specific artifact's SHA-256 digest.
6. Verify AArch64 Android ELF identity, size, and `--version` against the intended release as described in [`ANDROID_ARTIFACTS.md`](ANDROID_ARTIFACTS.md).
7. Install or upgrade through `scripts/termux_deploy.sh`.
8. Confirm deployment status, runit state, health, readiness, and authenticated discovery.
9. Validate representative allowed and denied MCP calls.
10. Exercise rollback before declaring production readiness.
11. Preserve the prior known-good release through sustained battery, thermal, and process-restriction validation.

Do not describe fixed server diagnostics as arbitrary command execution or exact-stream volume control as general Android authority. Shells, caller-selected commands, and unrelated high-impact tools remain unavailable until their independent gates, tests, audit behavior, and recovery semantics are complete.
