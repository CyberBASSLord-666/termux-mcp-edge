# Operations Guide

## Purpose

Termux MCP Edge runs as a small Rust/Axum service on Android through Termux. The supported production path uses versioned releases, the fixed `mcp_runtime` runit service, fail-closed bearer authentication, mobile-conscious request limits, exact transport allowlists, and safe-rooted filesystem tools.

## Baseline operating model

- Rust single binary.
- `GET /health` and `GET /ready` operational endpoints.
- Optional authenticated stable MCP 2025-11-25 `POST`, `GET`, and `DELETE /mcp` handling; GET returns 405 by default, with finite cursor replay available only through explicit SSE opt-in.
- Exact MCP order: authentication; early `Content-Length`, fail-fast concurrency, and total timeout enforcement; streaming body limiting and extraction; exact `Host`/`Origin`; method/media/protocol/session/grant validation; then JSON-RPC lifecycle, discovery, tools, and authorized mutations.
- Four concurrent authenticated MCP requests by default.
- Thirty-second request timeout by default.
- Two-MiB request-body ceiling by default.
- Versioned Termux release directories with atomic `current` and `previous` links.
- Fixed `mcp_runtime` runit service only.
- Dedicated safe-root defaults plus independent default-disabled directory, file-copy, and file-write mutation gates, with request-scoped authorization for each exact live mutation.

## Secure router construction and serving

The package binary and every downstream embedding use the one public [`McpRouterBuilder`](EMBEDDING.md). It requires sealed authentication, request-limit, transport-security, and filesystem-root policies. `new` validates the listener declaration and lifetime-pins every root; `build` initializes every requested optional client and installs the fixed layer order above. Raw state and legacy router constructors are not production API.

Complete builder construction before opening the TCP listener. Invalid listener text, a non-loopback declaration paired with unauthenticated development mode, empty/relative/filesystem-root/missing/non-directory/symlinked roots, an uncompiled requested capability, a mutation authority paired with unauthenticated policy, or an unavailable optional client returns a typed non-sensitive `McpRouterBuildError` instead of panicking or silently disabling the request.

Serve the returned router with `into_make_service_with_connect_info::<SocketAddr>()`. Static-token mode authenticates independently of peer metadata; explicit unauthenticated localhost mode requires both a loopback listener declaration and request-time `ConnectInfo` proving the actual TCP peer is IPv4 or IPv6 loopback. Missing metadata and non-loopback peers fail closed before request limits or body handling. Keep the builder declaration, actual listener bind, and `TransportSecurityPolicy` host/port consistent.

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
  http://127.0.0.1:8000/mcp | jq -e '.result.tools | length == 16'

rm -f "$MCP_RESPONSE_HEADERS"
unset MCP_TEST_TOKEN MCP_SESSION_ID MCP_RESPONSE_HEADERS
```

Do not enable shell tracing, echo token variables, or include credential-bearing commands in screenshots or issue text.

Each process holds at most 64 sessions and expires them after 30 idle minutes. Missing required post-initialize protocol/session headers return HTTP 400; expired, terminated, malformed, or unknown sessions return HTTP 404; capacity exhaustion returns HTTP 503. A client should DELETE a finished session and initialize a new session after HTTP 404 or a server restart. Session IDs do not replace the bearer token. SSE is disabled by default. If enabled, only finite request-response streams enter replay state, bounded to 8 streams and 256 KiB per session; terminate unused sessions promptly so their replay budget is released immediately.

## Enabling and issuing one `copy_file` mutation

Leave live copying disabled unless it is operationally required. Its private `runtime.env` posture is independent from directory creation, file writing, and Android volume control:

```dotenv
MCP__FILE__COPY_FILE_MUTATION_ENABLED=true
MCP__AUTH__STATIC_TOKEN=replace-with-a-strong-random-token
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

The gate defaults to `false`. Enabling it without `mcp-runtime`, static-token authentication, or the complete valid key pair fails startup. After initializing the target MCP session, issue a grant locally with the exact deployed binary; do not transmit the key or private inputs to MCP:

```bash
umask 077
COPY_GRANT_FILE="$(mktemp "$HOME/.termux-mcp-copy-grant.XXXXXX")"
chmod 600 "$COPY_GRANT_FILE"
MCP__CAPABILITY__CONFIG_FILE="$HOME/.config/termux-mcp-edge/runtime.env" \
MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
MCP__CAPABILITY__COPY_FILE_SOURCE="$HOME/mcp-files/source.bin" \
MCP__CAPABILITY__COPY_FILE_DESTINATION="$HOME/mcp-files/destination.bin" \
  "$HOME/.local/share/termux-mcp-edge/current/bin/termux-mcp-server" \
  --issue-copy-file-grant >"$COPY_GRANT_FILE"
```

Read the single grant line only while constructing the matching active-session `copy_file` mutation request, send it exactly once as `MCP-Capability-Grant`, and remove `COPY_GRANT_FILE` immediately after the attempt. The issuer independently opens and hashes the exact single-link source; the grant is invalid if either endpoint or the source identity/bytes changes. Never print, log, retain, or attach a grant. See [`COPY_FILE_CAPABILITY_GRANTS.md`](COPY_FILE_CAPABILITY_GRANTS.md) for the full issuance, rotation, denial, and replay contract.

## Enabling and issuing one `write_file` mutation

Leave live writing disabled unless it is operationally required. Its private `runtime.env` posture is independent from directory creation and Android volume control:

```dotenv
MCP__FILE__WRITE_MUTATION_ENABLED=true
MCP__AUTH__STATIC_TOKEN=replace-with-a-strong-random-token
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

The gate defaults to `false`. Enabling it on a binary without `mcp-runtime`, without static-token authentication, or without the complete valid key pair fails startup. Partial or malformed capability-key configuration also fails closed even when this specific mutation gate is off. After changing the private mode-`0600` environment file, restart the service and confirm `runtime_status.fileWriteMutationEnabled`, grant-required/header/TTL metadata, the 1 MiB file ceiling, and the 16 KiB response ceiling.

Issue each grant locally with the exact deployed binary after initializing the target MCP session. Put the exact intended UTF-8 bytes in an absolute private stable no-follow regular file:

```bash
umask 077
WRITE_GRANT_FILE="$(mktemp "$HOME/.termux-mcp-write-grant.XXXXXX")"
WRITE_CONTENT_FILE="$(mktemp "$HOME/.termux-mcp-write-content.XXXXXX")"
chmod 600 "$WRITE_GRANT_FILE" "$WRITE_CONTENT_FILE"
# Populate WRITE_CONTENT_FILE with exact intended bytes without shell tracing.

MCP__CAPABILITY__CONFIG_FILE="$HOME/.config/termux-mcp-edge/runtime.env" \
MCP__CAPABILITY__SESSION_ID="$MCP_SESSION_ID" \
MCP__CAPABILITY__WRITE_FILE_TARGET="$SAFE_ROOT_WRITE_TARGET" \
MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE="$WRITE_CONTENT_FILE" \
MCP__CAPABILITY__WRITE_FILE_DISPOSITION=create \
  "$HOME/.local/share/termux-mcp-edge/current/bin/termux-mcp-server" \
  --issue-write-file-grant >"$WRITE_GRANT_FILE"
```

Choose `create` only while the target is absent; choose `replace` only for the exact existing regular file. Send the single grant line only as `MCP-Capability-Grant` on the matching `write_file` `tools/call` with explicit `dry_run:false` and byte-for-byte identical JSON content. A grant does not authorize another principal, session, root, normalized path, content, disposition, or replacement inode. Preview does not consume it; live validation consumes it atomically immediately before publication work, and every later failure retains consumption. Remove both private files after the attempt and never copy the token, grant, content, path, digests, session, or filesystem identities into logs, tickets, screenshots, or release evidence.

The transport accepts exactly one bounded ASCII grant header only on an active-session `tools/call` for a grant-aware tool. Authentication, Host/Origin and method/media/header checks, lifecycle, exact tool context, closed schema, gate, complete-response preflight, safe-root/target classification, and grant binding all precede the first state change. Full transaction and rotation details are in [`WRITE_FILE_CAPABILITY_GRANTS.md`](WRITE_FILE_CAPABILITY_GRANTS.md).

## Request limits

The listener defaults to `MCP__SERVER__PORT=8000` and accepts only ports `1–65535`. Port `0`, malformed numbers, and present non-Unicode security/network configuration values fail before the listener starts. Only absent variables use defaults.

| Setting | Default | Valid range |
|---|---:|---:|
| `MCP__TRANSPORT__MAX_CONCURRENT_REQUESTS` | `4` | `1–64` |
| `MCP__TRANSPORT__REQUEST_TIMEOUT_SECONDS` | `30` | `1–300` |
| `MCP__TRANSPORT__MAX_BODY_BYTES` | `2097152` | `1024–8388608` |
| `MCP__TRANSPORT__SSE_ENABLED` | `false` | `true` or `false` |

Values outside these ranges fail startup. Increasing concurrency and body size together increases possible peak memory use; evaluate them together on the target device.

When SSE is enabled, JSON-RPC responses up to 128 KiB are retained for exact cursor resumption. Larger bounded responses remain JSON. The canonical serialized non-null JSON-RPC request id is limited to 1,048,576 bytes independently of the configured whole-request limit; larger ids receive HTTP 413 with a null response id before session allocation or method dispatch. Missing `Last-Event-ID` receives 405; malformed values receive 400; unavailable values receive 404. A reconnect must reuse the original session, bearer authentication, protocol version, Host, and Origin headers.

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
7. `find_paths` — case-sensitive literal basename discovery with exact kind/depth filters, descriptor-relative no-follow traversal, and content-free bounded results.
8. `hash_file` — streaming SHA-256 for one no-follow regular file up to 16 MiB, returning only digest and byte count.
9. `list_directory` — bounded safe-rooted listing.
10. `path_metadata` — bounded safe-rooted regular-file or directory metadata without content or host identifiers.
11. `read_binary_file` — one no-follow regular file up to 1 MiB as canonical padded base64, without path or host metadata.
12. `read_binary_range` — one byte range up to 256 KiB from a no-follow regular file up to 64 MiB as canonical padded base64, with explicit EOF metadata and no path or host metadata.
13. `read_file` — bounded safe-rooted UTF-8 reads.
14. `read_text_range` — one code-point-safe UTF-8 byte range up to 256 KiB from a no-follow regular file up to 64 MiB, with explicit continuation and EOF metadata and no path or host metadata.
15. `search_text` — bounded case-sensitive literal UTF-8 location search without content excerpts.
16. `write_file` — safe-rooted, 1 MiB UTF-8 preview by default; live mode-`0600` create/replace is independently disabled and exact-request-grant gated, with a content/path-free 16 KiB result that reports `recoveryArtifactRetained`.

An `android-battery-status` binary with `MCP__ANDROID__BATTERY_STATUS_ENABLED=true` additionally exposes `android_battery_status` as the seventeenth tool. It is disabled and hidden by default; see [`ANDROID_BATTERY_STATUS.md`](ANDROID_BATTERY_STATUS.md).

An `android-volume-status` binary with `MCP__ANDROID__VOLUME_STATUS_ENABLED=true` instead exposes `android_volume_status` as the seventeenth tool. It is independently disabled and hidden by default, uses only the fixed zero-argument `termux-volume` status mode, and never authorizes volume mutation; see [`ANDROID_VOLUME_STATUS.md`](ANDROID_VOLUME_STATUS.md). An all-feature validation build can expose both provider tools when both runtime flags are explicitly enabled.

An `android-volume-control` binary with `MCP__ANDROID__VOLUME_CONTROL_ENABLED=true`, static-token authentication, and the capability key pair exposes `set_android_volume`. It defaults to fresh validated preview. Explicit mutation requires one exact request grant and performs fixed execution, verification, and restoration on failure; see [`ANDROID_VOLUME_CONTROL.md`](ANDROID_VOLUME_CONTROL.md).

A `command-execution` binary with `MCP__COMMAND__ENABLED=true` exposes `run_command_profile` after the sixteen baseline tools. It offers only `server_version`, `server_help`, and `execution_boundary` against the attested already-loaded server image. The package binary uses the same public builder as embeddings, but alone can call its crate-private command-enablement setter; downstream crates are structurally command-disabled. Initialization matches the exact-name no-follow candidate to `/proc/self/exe` by device/inode and retains the first safe root by no-follow directory descriptor; children spawn `/proc/self/exe` with cwd `/proc/self/fd/<fd>`, empty environment, null stdin, and immutable maxima of 5 seconds, 16 KiB stdout, and 4 KiB stderr. It remains hidden when disabled; see [`command-execution-gate.md`](command-execution-gate.md). An all-feature validation build exposes twenty tools only when all four optional runtime flags are explicitly enabled.

The runtime does not expose Android platform control beyond exact request-authorized volume, an arbitrary shell or command runner, global process inventory, arbitrary service inspection, service mutation, package management, network mutation, or unrelated high-impact controls.

Filesystem responses have explicit mobile-oriented ceilings:

- `create_directory` validates one absent child by default. Explicit `dry_run:false` selects mutation but succeeds only when `MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true` and the request carries one unexpired, exact-target, single-use `MCP-Capability-Grant`. Confinement completes before authorization; consumption occurs immediately before the first mutation and survives downstream failure. The operation creates fixed mode `0700`, publishes without replacement, syncs child and parent descriptors, and caps the complete response at 16 KiB; see [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md) and [`SAFE_ROOT_DIRECTORY_CREATION.md`](SAFE_ROOT_DIRECTORY_CREATION.md).
- `write_file` validates at most 1 MiB of UTF-8 and classifies an absent target as `create` or an existing no-follow regular file as `replace`. Explicit `dry_run:false` succeeds only when `MCP__FILE__WRITE_MUTATION_ENABLED=true`, static authentication and the capability key pair are active, and one unexpired single-use grant matches the principal, session, root, normalized target, exact content, disposition, and exact old identity for replace. It creates a mode-`0600` randomized staging entry in the target parent's reserved private quarantine. Create publishes with atomic `NOREPLACE` and retains no artifact. Replace accepts only a single-link regular target of at most 1 MiB, performs one irreversible `EXCHANGE`, verifies the exact staged inode at the target, and preserves the displaced prior inode/content as recovery material. The complete 16 KiB result exposes no path, content, or artifact name and includes `recoveryArtifactRetained`; see [`WRITE_FILE_CAPABILITY_GRANTS.md`](WRITE_FILE_CAPABILITY_GRANTS.md).
- `copy_file` validates one regular source and absent destination by default. Explicit `dry_run:false` succeeds only when `MCP__FILE__COPY_FILE_MUTATION_ENABLED=true`, static authentication and the capability key pair are active, and one unexpired single-use grant matches the principal, session, both roots and normalized paths, exact source identity/size/high-resolution ctime/SHA-256, absent destination, and no-replace posture. It copies at most 1 MiB from the exact held source descriptor, stages fixed mode `0600` in the hidden quarantine, publishes atomically without replacement, verifies exact bytes and identities, syncs file and parent descriptors, returns neither endpoint path nor content, and caps the complete response at 16 KiB; see [`COPY_FILE_CAPABILITY_GRANTS.md`](COPY_FILE_CAPABILITY_GRANTS.md) and [`SAFE_ROOT_FILE_COPY.md`](SAFE_ROOT_FILE_COPY.md).
- `find_paths` accepts one case-sensitive literal basename query of at most 256 UTF-8 bytes, traverses no-follow descriptors to depth 5, examines at most 8,192 entries, returns at most 512 lexicographically ordered file/directory matches, and caps the complete response at 262,144 bytes; see [`SAFE_ROOT_PATH_DISCOVERY.md`](SAFE_ROOT_PATH_DISCOVERY.md).
- `hash_file` streams at most 16 MiB from one exact held no-follow regular-file descriptor through SHA-256, rejects growth past the limit, returns only lowercase digest and byte count, and caps the complete response at 16 KiB before the file read; see [`SAFE_ROOT_FILE_HASHING.md`](SAFE_ROOT_FILE_HASHING.md).
- `read_binary_file` reads at most 1 MiB from one exact held no-follow regular-file descriptor, rejects runtime growth, returns canonical padded RFC 4648 base64 without path or host metadata, and preflights the complete 1,507,328-byte response ceiling before file access; see [`SAFE_ROOT_BINARY_READS.md`](SAFE_ROOT_BINARY_READS.md).
- `read_binary_range` reads at most 256 KiB from one exact held no-follow regular-file descriptor up to 64 MiB, accepts offset equal to EOF as an empty result, rejects offset past EOF and concurrent size change, and preflights the complete 393,216-byte response ceiling before file access; see [`SAFE_ROOT_BINARY_RANGES.md`](SAFE_ROOT_BINARY_RANGES.md).
- `list_directory` returns at most 4,096 entries and at most 256 KiB for the complete JSON-RPC response. Entries are ordered deterministically by path before publication. `structuredContent.truncated` reports when either ceiling prevented a complete result and the response publishes both limits.
- `path_metadata` returns exactly normalized path, regular-file/directory kind, nullable file size, nullable RFC 3339 modification time, and the fixed 16 KiB full-response ceiling. It does not return content, inode/device/UID/GID/mode/access-time data, link targets, or unsupported object types; see [`SAFE_ROOT_PATH_METADATA.md`](SAFE_ROOT_PATH_METADATA.md).
- `read_file` reads at most 1 MiB of valid UTF-8 and caps the complete JSON-RPC response at 1,114,112 bytes. The file content appears once in `structuredContent.content`; the text content is a fixed-format byte-count summary. JSON escaping that would exceed the response ceiling is rejected with a bounded payload-too-large error.
- `read_text_range` reads one code-point-aligned UTF-8 range of 4 to 262,144 requested bytes from an exact held no-follow regular-file descriptor up to 64 MiB. It defers a partial trailing code point, returns `nextOffsetBytes` for lossless pagination, rejects midpoint offsets, invalid/truncated UTF-8, offset past EOF, and concurrent size change, and preflights the complete 1,703,936-byte worst-case escaped response before file access; see [`SAFE_ROOT_TEXT_RANGES.md`](SAFE_ROOT_TEXT_RANGES.md).
- `search_text` accepts one literal query of at most 256 UTF-8 bytes, examines at most 8,192 entries and 4,096 files through depth 5, reads at most 1 MiB per file and 8 MiB total, returns at most 256 path/line/byte-column matches, and caps the complete response at 256 KiB. It returns no file-content excerpt or query echo; see [`SAFE_ROOT_TEXT_SEARCH.md`](SAFE_ROOT_TEXT_SEARCH.md).

These response ceilings are independent of the authenticated request-body ceiling and cannot be increased through environment configuration.

## Filesystem safe roots

The default filesystem root is:

```text
/data/data/com.termux/files/home/mcp-files
```

Keep configured roots limited to dedicated project directories. Configuration accepts one through 64 entries and deterministically normalizes, sorts, and deduplicates valid labels. Empty or relative entries, filesystem root `/`, traversal, missing/non-directory objects, and symlinks in a root or any ancestor are rejected before the listener opens. Broad shared Android storage is not a default.

Fallible startup pins every distinct normalized root label with a retained no-follow directory descriptor and device/inode identity. Tool clones share those pins. Each descendant operation duplicates and verifies the selected descriptor, then walks below it with descriptor-relative no-follow operations; configured path labels are selection metadata, not authority, and are never reopened for a live request. Renaming or replacing a root or ancestor cannot redirect the running service: it continues against the original pinned directory and leaves the pathname replacement untouched. Offline grant issuers pin independently and sign the same identity contract, so issuance against a later replacement does not authorize the runtime's original root.

Restart is the explicit repinning boundary. To change, remount, or replace a configured storage hierarchy, quiesce clients and all same-UID writers, stop the service, make the change, restart, and verify runit state, health, readiness, pinned-root count, and representative filesystem calls. Failures, debug output, aggregate audits, and retained evidence expose no configured-root paths or descriptor/device/inode metadata.

Symlinks are not a supported aliasing mechanism inside a safe root. File writes use the target parent's fixed `.termux-mcp-write-quarantine`; this mode-`0700` namespace is hidden from and rejected by every MCP filesystem operation. Randomized staging entries start at mode `0600`, are synchronized and verified before publication, and create never replaces. Replace uses one irreversible exchange and preserves the displaced prior inode/content and metadata under its randomized artifact name. It does not unlink, truncate, chmod, or swap that object back after capture. An error after the exchange may leave the authorized new inode at the public target with the displaced object quarantined; the call is still denied and the grant remains consumed.

The quarantine is limited to 32 regular artifacts and 32 MiB per target parent. Its advisory lock is nonblocking and coordinates cooperating runtime writers only; the cap is not a global disk bound, and another process under the same Unix UID can cause contention or denial of service. Capacity, mode, entry-shape, or lock failures deny the write without disclosing private names or counts.

### Write recovery-artifact maintenance

Never remove recovery artifacts while the service or another same-UID writer is active. Quiesce clients, stop the service and other same-UID writers, inspect the specific target parent's quarantine locally, and manually preserve or remove only selected `.termux-mcp-write-artifact-*` entries. Avoid recursive deletion and broad globs. Restart the service, then confirm runit state, health, and readiness before accepting writes again. The artifacts contain prior file content and rely on the mode-`0700` directory and Unix ownership for confidentiality; copy required recovery material to independent protected storage before deletion.

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
9. Validate representative allowed and denied MCP calls, including the independent disabled/enabled file-write gate, exact-binary grant issuance, authorized create/replace, mismatch/replay denials, fixed mode/limits, no-replace creation, irreversible exchange, bounded retained recovery artifacts, reserved-namespace isolation, and private audit/result surfaces.
10. Exercise rollback before declaring production readiness.
11. Preserve the prior known-good release through sustained battery, thermal, and process-restriction validation.

Do not describe fixed server diagnostics as arbitrary command execution or exact-stream volume control as general Android authority. Shells, caller-selected commands, and unrelated high-impact tools remain unavailable until their independent gates, tests, audit behavior, and recovery semantics are complete.
