# Capability Catalog

Termux MCP Edge separates capability into three independent layers:

1. **Compile-time inclusion** determines which code exists in the binary.
2. **Runtime enablement** determines whether an optional tool is discoverable and callable.
3. **Request authorization** is required for each live mutation. A runtime flag never replaces an operation-bound, single-use grant.

All MCP tools also require an initialized, protected MCP transport. Normal deployments use static-token authentication; the only exception is the explicitly enabled, socket-verified localhost development posture. Filesystem tools remain confined to configured safe roots, and every mutation defaults to preview.

## Build choices

| Build goal | Cargo selection | MCP tools after normal runtime enablement | Release contract |
|---|---|---:|---|
| Health and readiness only | default features | 0 | Isolated posture |
| Baseline MCP server | `--features mcp-runtime` | 17 | Isolated posture |
| Baseline plus battery status | `--features android-battery-status` | 18 when its runtime flag is enabled | Isolated posture |
| Baseline plus volume status | `--features android-volume-status` | 18 when its runtime flag is enabled | Isolated posture |
| Baseline plus volume control | `--features android-volume-control` | 18 for control alone, or 19 when volume-status discovery is also enabled | Isolated posture |
| Baseline plus fixed diagnostics | `--features command-execution` | 18 when its runtime flag is enabled | Isolated posture |
| Governed aggregate | `--features full-suite` | Exactly 17 with optional flags off; exactly 21 when all four are enabled | Named public aggregate posture |
| Raw compatibility build | `--all-features` | Up to 21 | Development/compatibility coverage only; not a release artifact |

The four optional runtime flags are:

```dotenv
MCP__ANDROID__BATTERY_STATUS_ENABLED=true
MCP__ANDROID__VOLUME_STATUS_ENABLED=true
MCP__ANDROID__VOLUME_CONTROL_ENABLED=true
MCP__COMMAND__ENABLED=true
```

Compiling a feature does not turn on its runtime flag. `full-suite` is an explicit alias for `mcp-runtime`, `android-battery-status`, `android-volume-status`, `android-volume-control`, and `command-execution`; it adds no master runtime gate and no authorization bypass. The `android-volume-control` feature includes the volume-status implementation needed for validation and recovery, but it does not make `android_volume_status` discoverable unless `MCP__ANDROID__VOLUME_STATUS_ENABLED=true` is also configured.

The release contract validates seven Android artifacts: six least-privilege postures and the named `full-suite` aggregate. Raw `--all-features` remains useful for compatibility testing, but it must not be renamed or represented as the aggregate durable release asset. See [Android validation artifacts](ANDROID_ARTIFACTS.md).

## Baseline MCP tools

Every tool in this table is compiled by `mcp-runtime` and appears in protected discovery after the MCP session is initialized. Runtime mutation flags change authorization posture; they do not add or remove these 17 tool names.

| Tool | Class | Purpose | Runtime and request authority |
|---|---|---|---|
| `runtime_status` | Status | Reports bounded runtime posture and aggregate, non-sensitive audit counters. | Read-only; no additional runtime flag or grant. |
| `platform_info` | Status | Reports non-sensitive platform metadata. | Read-only; no additional runtime flag or grant. |
| `android_status` | Status | Reports allowlisted Android and Termux status metadata. | Read-only; no additional runtime flag or grant. |
| `project_service_status` | Status | Reports allowlisted state for the project-owned `mcp_runtime` service. | Read-only; no arbitrary service selection. |
| `create_directory` | Filesystem mutation | Validates or creates one absent directory at fixed mode `0700`, without creating parents or replacing an entry. | Preview by default. Live use requires `MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED=true` and an exact single-use create grant. |
| `copy_file` | Filesystem mutation | Validates or copies one regular file of at most 1 MiB to an absent destination at fixed mode `0600`. | Preview by default. Live use requires `MCP__FILE__COPY_FILE_MUTATION_ENABLED=true` and an exact source/content/destination-bound grant. |
| `trash_file` | Filesystem mutation | Validates or reversibly moves one single-link regular file of at most 1 MiB into private bounded recovery storage. | Preview by default. Live use requires `MCP__FILE__TRASH_FILE_MUTATION_ENABLED=true` and an exact identity/content/recovery-bound grant. No MCP purge or restore exists. |
| `find_paths` | Filesystem read | Finds bounded, ordered literal basename matches without reading file content. | Read-only and safe-rooted; no additional runtime flag or grant. |
| `hash_file` | Filesystem read | Streams SHA-256 for one regular file of at most 16 MiB and returns only the digest and byte count. | Read-only and safe-rooted; no additional runtime flag or grant. |
| `list_directory` | Filesystem read | Returns a deterministic, response-bounded safe-root directory listing. | Read-only and safe-rooted; no additional runtime flag or grant. |
| `path_metadata` | Filesystem read | Returns bounded regular-file or directory metadata without host identifiers or content. | Read-only and safe-rooted; no additional runtime flag or grant. |
| `read_binary_file` | Filesystem read | Returns one regular file of at most 1 MiB as canonical padded base64. | Read-only and safe-rooted; no additional runtime flag or grant. |
| `read_binary_range` | Filesystem read | Returns at most 256 KiB from a regular file of at most 64 MiB as canonical padded base64. | Read-only and safe-rooted; no additional runtime flag or grant. |
| `read_file` | Filesystem read | Returns one bounded valid UTF-8 file. | Read-only and safe-rooted; no additional runtime flag or grant. |
| `read_text_range` | Filesystem read | Returns a code-point-safe UTF-8 range with continuation and EOF metadata. | Read-only and safe-rooted; no additional runtime flag or grant. |
| `search_text` | Filesystem read | Returns bounded locations for a case-sensitive literal UTF-8 query, without content excerpts. | Read-only and safe-rooted; no additional runtime flag or grant. |
| `write_file` | Filesystem mutation | Validates or writes at most 1 MiB of UTF-8 as fixed mode `0600`; replacement retains the displaced object in private bounded recovery storage. | Preview by default. Live use requires `MCP__FILE__WRITE_MUTATION_ENABLED=true` and an exact content/disposition/target-identity grant. |

Detailed request schemas and fixed resource ceilings are advertised by `tools/list` and documented in the linked contracts from the [operations guide](OPERATIONS.md#current-mcp-tools).

## Optional MCP tools

| Tool | Compile feature | Runtime discovery gate | Purpose | Live request grant |
|---|---|---|---|---|
| `android_battery_status` | `android-battery-status` | `MCP__ANDROID__BATTERY_STATUS_ENABLED=true` | Bounded read-only Termux:API battery telemetry. | None. |
| `android_volume_status` | `android-volume-status` | `MCP__ANDROID__VOLUME_STATUS_ENABLED=true` | Bounded read-only status for the six supported Android audio streams. | None. |
| `set_android_volume` | `android-volume-control` | `MCP__ANDROID__VOLUME_CONTROL_ENABLED=true` | Previews or sets one supported audio stream to one validated level, then verifies the result and attempts restoration on failure. | Required for every `dry_run:false` call. Static-token authentication and capability-key configuration are also mandatory. |
| `run_command_profile` | `command-execution` | `MCP__COMMAND__ENABLED=true` | Runs only `server_version`, `server_help`, or `execution_boundary` against the attested server executable. | None. The caller cannot supply a program, argv, environment, path, timeout, or limit. |

Battery and volume tools require the official Termux:API add-on and package. The fixed-command posture is not arbitrary command execution.

## Live mutation authorization

All five live mutation families use the same public header name, but their grants are cryptographically separate and cannot authorize one another:

```http
MCP-Capability-Grant: <single-use-operation-bound-grant>
```

| Tool | Default-disabled runtime gate | Exact local issuer | Grant binds |
|---|---|---|---|
| `create_directory` | `MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED` | `--issue-create-directory-grant` | Principal, session, pinned root, absent target, and mutation posture. |
| `copy_file` | `MCP__FILE__COPY_FILE_MUTATION_ENABLED` | `--issue-copy-file-grant` | Principal, session, source identity and bytes, both paths and roots, absent destination, and no-replace posture. |
| `trash_file` | `MCP__FILE__TRASH_FILE_MUTATION_ENABLED` | `--issue-trash-file-grant` | Principal, session, target identity and bytes, pinned root and path, and recovery-retained posture. |
| `write_file` | `MCP__FILE__WRITE_MUTATION_ENABLED` | `--issue-write-file-grant` | Principal, session, target, exact content, create-or-replace disposition, and replacement identity when applicable. |
| `set_android_volume` | `MCP__ANDROID__VOLUME_CONTROL_ENABLED` | `--issue-android-volume-grant` | Principal, session, exact audio stream, and exact level. |

The runtime gates above require static-token authentication plus:

```dotenv
MCP__CAPABILITY__KEY_ID=primary-1
MCP__CAPABILITY__HMAC_KEY_HEX=replace-with-64-lowercase-hex-characters
```

A feature being compiled, a runtime gate being enabled, an authenticated session, and `dry_run:false` are all insufficient without the matching fresh grant. Grants normally expire after 60 seconds, are single-use within the supported one-process authority boundary, and are never MCP tool arguments.

For issuance and recovery details, use the dedicated contracts for [directory creation](CREATE_DIRECTORY_CAPABILITY_GRANTS.md), [file copy](COPY_FILE_CAPABILITY_GRANTS.md), [file trashing](TRASH_FILE_CAPABILITY_GRANTS.md), [file writes](WRITE_FILE_CAPABILITY_GRANTS.md), and [Android volume control](ANDROID_VOLUME_CONTROL.md).

## Deliberately unavailable authority

No build exposes a general shell, caller-selected command execution, global process inventory, arbitrary service control, package management, network mutation, recursive deletion, broad Android control, or unrestricted shared-storage access. Adding one of those authority classes requires a separate threat model, gate, tests, and release evidence; neither `full-suite` nor raw `--all-features` bypasses that boundary.
