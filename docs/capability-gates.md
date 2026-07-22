# Staged capability gates

The MCP runtime expands capability only through small, current-base pull requests with explicit scope, tests, and audit coverage. Every gate must preserve the existing default-deny posture unless the gate explicitly changes that posture and the change is covered by tests.

Compile composition does not collapse these gates. The named `full-suite` feature compiles every currently supported optional provider, yet discovery remains at exactly 17 tools until battery, volume-status, volume-control, and fixed-command runtime flags are enabled independently; all four produce exactly 21. Filesystem and volume mutations still require their own default-disabled gate and exact-operation single-use grant. Raw Cargo `--all-features` is development compatibility coverage, not a public release posture.

## Current baseline

Enabled staged tools:

- `runtime_status`
- `platform_info`
- `android_status`
- `project_service_status`
- `create_directory` preview for exactly one absent safe-rooted directory; mutation additionally requires the default-disabled runtime gate and one request-scoped single-use grant, then uses fixed mode `0700` and atomic no-replace publication
- `copy_file` preview for exactly one single-link no-follow regular source of at most 1 MiB and one absent safe-rooted destination; mutation is independently default-disabled, exact principal/session/root/path/source-identity/size/SHA-256/destination grant-gated, fixed mode `0600`, path/content-private, hidden-staged, and atomically no-replace published
- `trash_file` preview for exactly one single-link no-follow safe-rooted regular file of at most 1 MiB; live recovery retention is independently default-disabled, exact principal/session/root/path/identity/size/high-resolution-ctime/SHA-256 grant-gated, path/content/artifact-private, and atomically no-replace moved into its separate hidden bounded quarantine
- `find_paths` for case-sensitive literal basename discovery across at most 8,192 descriptor-relative no-follow entries to depth 5, with exact kind filtering, at most 512 ordered content-free matches, and a fixed complete-response ceiling
- `hash_file` for streaming SHA-256 of exactly one no-follow safe-rooted regular file of at most 16 MiB, with a digest-and-size-only response and content/path-private audit surfaces
- `list_directory`
- `path_metadata` for one descriptor-relative regular-file or directory metadata result without host identifiers
- `read_binary_file` for one no-follow safe-rooted regular file of at most 1 MiB, returned as canonical padded base64 without path or host metadata under a fixed complete-response ceiling
- `read_binary_range` for one range of at most 256 KiB from a no-follow safe-rooted regular file of at most 64 MiB, returned as canonical padded base64 with explicit EOF and fixed response metadata
- `read_file`
- `read_text_range` for one 4-to-256 KiB UTF-8 byte range from a no-follow safe-rooted regular file of at most 64 MiB, returned only on code-point boundaries with explicit next-offset and EOF metadata
- `search_text` for bounded literal UTF-8 location search without content excerpts
- `write_file` preview for one bounded UTF-8 target; mutation is independently default-disabled and additionally requires `MCP__FILE__WRITE_MUTATION_ENABLED=true`, static authentication, a capability key pair, and one request-scoped single-use grant bound to the exact content and `create`/`replace` target state. Create is atomic no-replace; replace retains the displaced object in a bounded private recovery quarantine.

Separately gated read-only tool:

- `android_battery_status` only in an `android-battery-status` build with `MCP__ANDROID__BATTERY_STATUS_ENABLED=true`
- `android_volume_status` only in an `android-volume-status` build with `MCP__ANDROID__VOLUME_STATUS_ENABLED=true`
- `run_command_profile` only in a `command-execution` build with `MCP__COMMAND__ENABLED=true`; this is a closed set of fixed read-only server diagnostics, not arbitrary command execution

Separately gated high-impact tool:

- `set_android_volume` only in an `android-volume-control` build with `MCP__ANDROID__VOLUME_CONTROL_ENABLED=true`, static-token authentication, and capability key configuration; it previews by default and every mutation requires one exact single-use request grant

Current audit visibility is aggregate and in-memory. The staged runtime exposes backend-neutral `auditCounters` through `runtime_status` for the currently wired status and filesystem surfaces. These counters are intentionally not retained request logs and store only stable tool names, gate names, modes, reason codes, and allowed or denied counts.

Still disabled:

- Android platform or audio control beyond the exact request-authorized volume capability
- Shell, arbitrary commands, caller-selected executables/argv, and all command mutation
- Global process listing and arbitrary service inspection
- Service mutation or control
- High-impact device or host controls

### `create_directory` request grant

Status: implemented as a narrowly scoped Class 2 authorization layer.

- Runtime gate: `MCP__FILE__CREATE_DIRECTORY_MUTATION_ENABLED`, default `false`.
- Key configuration: one paired lowercase key ID and 32-byte HMAC-SHA-256 key; static-token authentication is mandatory.
- Issuance: local exact-binary `--issue-create-directory-grant`, never an MCP tool.
- Transport: exactly one bounded `MCP-Capability-Grant` header, accepted only on `tools/call` for `create_directory`.
- Binding: static principal, canonical session UUID, capability, root device/inode, normalized target digest, mutating posture, format/key ID, JTI, issuance, and expiry.
- Consumption: atomic immediately before the first mutation attempt; dry runs and pre-authorization failures do not consume, while every later failure retains consumption.
- Lifetime/state: 60-second grants, five-second future skew, 120-second hard lifetime ceiling, and 4,096 unexpired replay entries.
- Privacy: no key, grant, fingerprint, session, JTI, target digest, path, or timestamp in responses, logs, or audit labels.

The complete contract is [`CREATE_DIRECTORY_CAPABILITY_GRANTS.md`](CREATE_DIRECTORY_CAPABILITY_GRANTS.md).

### `copy_file` request grant

Status: implemented as an independent narrowly scoped Class 2 authorization layer.

- Runtime gate: `MCP__FILE__COPY_FILE_MUTATION_ENABLED`, default `false`; it does not inherit create, write, or Android-volume enablement.
- Startup posture: an `mcp-runtime` binary, static-token authentication, and the exact paired lowercase key ID plus 32-byte HMAC key are mandatory.
- Issuance: local exact-binary `--issue-copy-file-grant`, never an MCP tool. The issuer receives the active session and private source/destination paths through `MCP__CAPABILITY__SESSION_ID`, `MCP__CAPABILITY__COPY_FILE_SOURCE`, and `MCP__CAPABILITY__COPY_FILE_DESTINATION`, then independently inspects and hashes the source.
- Transport: exactly one bounded ASCII `MCP-Capability-Grant` header, accepted only for an active-session live `copy_file` call. Preview, discovery, initialization, notifications, responses, GET, DELETE, and unrelated tools reject or ignore no grant context and never consume one.
- Binding: static principal, canonical session, capability code `4`, both anchored root identities and normalized paths, source device/inode/size/high-resolution ctime/one-link identity, SHA-256 of the exact bytes, fixed absent-destination/no-replace posture, format/key ID, JTI, issuance, and expiry. The fixed 65-byte payload serializes only JTI, family byte, keyed opaque operation binding, and timestamps.
- Ordering: response preflight with the real request id, fail-fast worker admission, descriptor preparation, process-lock acquisition, source/content/destination revalidation, hidden-quarantine capacity, and cancellation ownership all precede atomic grant consumption. Consumption immediately precedes the first staging mutation and survives every later outcome.
- Transaction: randomized mode-`0600` staging occurs only inside the mode-`0700` MCP-hidden write quarantine. Atomic `NOREPLACE`, held/named identity verification, destination-parent and quarantine sync, and identity-safe cleanup run while the create/copy/trash/write process lock remains held. Reversible trashing uses its separate hidden quarantine and retains the exact authorized inode instead of staging content.
- Result: only `dryRun`, byte count, fixed mode, and fixed 1 MiB/16 KiB limits; never a source or destination path, content, digest, identity, grant, or staging name.
- Lifetime/state: 60-second grants, five-second future skew, 120-second hard lifetime ceiling, 4,096 unexpired replay entries, and the shared bounded process-global authority registry.

The complete contract is [`COPY_FILE_CAPABILITY_GRANTS.md`](COPY_FILE_CAPABILITY_GRANTS.md).

### `trash_file` request grant

Status: implemented as an independent narrowly scoped Class 2 authorization layer.

- Runtime gate: `MCP__FILE__TRASH_FILE_MUTATION_ENABLED`, default `false`; it does not inherit create, copy, write, or Android-volume enablement.
- Startup posture: an `mcp-runtime` binary, static-token authentication, and the exact paired lowercase key ID plus 32-byte HMAC key are mandatory.
- Issuance: local exact-binary `--issue-trash-file-grant`, never an MCP tool. The issuer receives the active session and private target through `MCP__CAPABILITY__SESSION_ID` and `MCP__CAPABILITY__TRASH_FILE_TARGET`, then independently opens, classifies, and hashes the exact target.
- Transport: exactly one bounded ASCII `MCP-Capability-Grant` header, accepted only for an active-session live `trash_file` call. Preview, discovery, initialization, notifications, responses, GET, DELETE, and unrelated tools cannot consume it.
- Binding: static principal, canonical session, capability code `5`, anchored root identity, normalized target, exact device/inode/size/high-resolution ctime/one-link identity, SHA-256 of the exact bytes, and fixed recovery-retained posture. The fixed 65-byte payload serializes only JTI, family byte, keyed opaque operation binding, and timestamps.
- Ordering: complete 16 KiB response preflight, fail-fast worker admission, descriptor preparation, process-lock acquisition, target identity/content revalidation, separate trash-quarantine capacity, and cancellation ownership all precede atomic grant consumption. Consumption immediately precedes the first namespace mutation and survives every later outcome.
- Transaction: the runtime moves the exact authorized inode with descriptor-relative atomic `NOREPLACE` into an unpredictable name under `.termux-mcp-trash-quarantine`, verifies retained identity and content, and syncs both directories. It never unlinks, purges, overwrites, recursively removes, or exposes a restore/purge MCP surface.
- Result: only `dryRun`, byte count, `recoveryArtifactRetained`, and fixed limits; never a target path, content, digest, identity, grant, quarantine path, or artifact name.
- Bounds/state: one file up to 1 MiB, at most 32 retained regular artifacts and 32 MiB per parent, 60-second grants, five-second future skew, 120-second hard lifetime ceiling, 4,096 unexpired replay entries, and the shared bounded process-global authority registry.

The complete contract is [`TRASH_FILE_CAPABILITY_GRANTS.md`](TRASH_FILE_CAPABILITY_GRANTS.md).

### `write_file` request grant

Status: implemented as an independent narrowly scoped Class 2 authorization layer.

- Runtime gate: `MCP__FILE__WRITE_MUTATION_ENABLED`, default `false`; it does not inherit the directory-creation, file-copy, or Android-volume gate.
- Startup posture: an `mcp-runtime` binary, static-token authentication, and one paired lowercase key ID plus 32-byte HMAC-SHA-256 key are mandatory; partial or malformed key configuration fails closed.
- Issuance: local exact-binary `--issue-write-file-grant`, never an MCP tool. The issuer receives the active session, safe-root target, a private stable no-follow content file of at most 1 MiB, and exact `create` or `replace` disposition through `MCP__CAPABILITY__SESSION_ID`, `MCP__CAPABILITY__WRITE_FILE_TARGET`, `MCP__CAPABILITY__WRITE_FILE_CONTENT_FILE`, and `MCP__CAPABILITY__WRITE_FILE_DISPOSITION`.
- Transport: exactly one bounded ASCII `MCP-Capability-Grant` header, accepted only on an active-session `tools/call` for `write_file` or another explicitly grant-aware tool; it is rejected on initialization, notifications, responses, discovery, GET, DELETE, and unrelated tools.
- Binding: static principal, canonical session UUID, capability, safe-root device/inode, normalized target digest, exact content digest, exact `create`/`replace` disposition, and—on replacement—the existing target device, inode, size, high-resolution ctime, and one-link count, plus mutating posture. The fixed 65-byte payload (130 lowercase hex) carries only a random JTI, the signed write-family byte, keyed domain-separated operation binding, and issued/expiry timestamps; it does not serialize those binding inputs.
- Ordering: request authentication remains outermost. The runtime then enforces transport/header context, lifecycle and exact tool context, closed schema, the default-disabled gate, complete 16 KiB response preflight, 1 MiB payload and safe-root/target validation, recovery-quarantine capacity, and exact grant matching before atomically consuming the grant immediately before publication work.
- Transaction: each target parent reserves `.termux-mcp-write-quarantine`, which is mode `0700` and inaccessible through every MCP filesystem operation. The runtime creates one randomized mode-`0600` `.termux-mcp-write-artifact-*` staging entry there. Create publishes it with `NOREPLACE` and retains no artifact. Replace accepts only a single-link regular target of at most 1 MiB, performs one irreversible `EXCHANGE`, verifies the exact staged inode at the final name, and leaves the displaced prior inode/content under the randomized quarantine name. No automatic rollback or destructive post-capture cleanup is attempted.
- Bounds/concurrency: each parent retains at most 32 artifacts and 32 MiB. The advisory quarantine lock is nonblocking and coordinates only cooperating writers; the limit is not global, and a same-UID peer can cause a bounded denial or denial of service. A post-commit failure may leave the authorized new inode at the target and the displaced object quarantined, with the grant consumed.
- Result: success returns only `dryRun`, byte count, disposition, `recoveryArtifactRetained`, fixed mode, and fixed file/response limits. Replacement reports `true`; create and preview report `false`. It never returns the path, content, or artifact name, and the complete response is capped at 16 KiB.
- Consumption/state: preview and pre-authorization failures do not consume; every failure after consumption retains it. Grants normally live 60 seconds, permit five seconds of future skew, have a 120-second hard lifetime ceiling, and use at most 4,096 unexpired replay entries.
- Privacy: responses and aggregate audits contain only stable tool/gate/mode/decision/reason labels. They never retain the key, header, principal fingerprint, session, JTI, target/content digest, disposition-specific identity, path, content, artifact name, retained counts/bytes, or timestamp.

The complete contract is [`WRITE_FILE_CAPABILITY_GRANTS.md`](WRITE_FILE_CAPABILITY_GRANTS.md).

### Process-local request-grant state

Every live request-grant family resolves replay, clock-rollback, and replay-capacity state through one bounded process-global registry. Independently constructed authorities share that state only when their capability family, key identifier, HMAC key, and static authenticated principal are all identical. A token consumed through one equivalent router or authority is therefore replayed through every other equivalent authority in the same server process, including under concurrent use. Different families, keys, key identifiers, and principals remain isolated. Registry or namespace lock poisoning, an invalid zero capacity, and exhaustion of the bounded namespace registry fail closed without exposing a key, principal fingerprint, or namespace identifier.

This is a process boundary, not a distributed replay service. Separate operating-system processes do not share consumed grant identifiers or last-observed clocks even when configured with the same key material. Production deployments must run one grant-consuming service process for a capability-key/principal domain, or provide an external atomic one-use coordinator before introducing multiple consumers. Restart clears the in-memory state; rotate the key during restart when immediate invalidation of outstanding grants is required.

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

Status: implemented for static read-only allowlisted Android/Termux status metadata and separately gated read-only battery and volume telemetry. Exact-stream control is isolated in Gate 5.

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
- Any mutation or device-control action through the read-only status gate
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
- Binary-crate-only command enablement; the single public builder defaults disabled and exposes no enabling method, with dependency and selected-workspace compile probes
- Exact-name candidate opened no-follow and matched by device/inode to an independently opened `/proc/self/exe`; later launches use only `/proc/self/exe`
- Fixed complete argv for `server_version`, `server_help`, and `execution_boundary`
- First canonical configured safe root retained by no-follow directory descriptor, filesystem-root aliases rejected by device/inode, and child cwd selected through `/proc/self/fd/<fd>`
- Empty environment, null stdin, immutable 5-second/16 KiB stdout/4 KiB stderr maxima, and two non-queueing concurrency permits
- The cancellation-safe shared process supervisor with process-group isolation, immediate termination, cleanup reserve, and authoritative direct-child reaping
- UTF-8 and zero-exit success requirements; stable non-sensitive failures with no partial output
- Hidden disabled discovery, runtime-disabled direct-call denial, and aggregate audit counters using only reason codes and numeric profile ordinals
- Exact-source command artifact within the seven-artifact matrix and strict-v2 native ARM64 official-Termux validation with exactly 29 MCP requests plus a separate typed wrong-name construction-failure phase, executable/cwd pathname replacement, pre-service rejection and redaction evidence, and complete provenance/artifact/environment checks

Denied:

- Raw command strings, shells, interpreters, caller-selected programs, argv, paths, environment, stdin, timeouts, or limits
- Profiles with placeholders, credentials, broad host inspection, filesystem mutation, Android control, service/package/process/network mutation, or other side effects
- Raw output or caller values in audit counters

Required before any future expansion:

- Apply the full rejection checklist in [`command-profile-validation.md`](command-profile-validation.md)
- Keep each new capability in a separately reviewed profile or higher-risk gate
- Preserve deterministic native evidence and exact-head CI/Security/Android success
- Never redefine fixed diagnostics as arbitrary or high-impact execution

## Gate 5: request-authorized Android volume control

Status: one exact-stream control is implemented behind independent compile/runtime/auth/key/request gates.

Implemented scope:

- Separate `android-volume-control` feature and default-false `MCP__ANDROID__VOLUME_CONTROL_ENABLED` flag
- Closed `set_android_volume` schema with the six documented streams, integer level, and preview-first posture
- Fresh strict status bounds before preview or mutation
- Exact-binary offline 60-second grant issuance bound to principal/session/capability/stream/level/posture
- Atomic single-use replay state and header-context enforcement
- One non-queueing mutation permit
- Fixed absolute program, two arguments, root cwd, empty environment, null stdin, and fixed supervisor bounds
- Post-mutation status verification plus automatic restoration and confirmed/unconfirmed stable outcomes
- Recovery ownership independent from request cancellation after grant consumption
- Aggregate private audit counters and exact artifact/native evidence

The complete contract is [`ANDROID_VOLUME_CONTROL.md`](ANDROID_VOLUME_CONTROL.md).

## Gate 6: other high-impact controls

Status: threat model complete; all other high-impact controls remain disabled.

Examples:

- Package installation or removal
- Service restart or stop
- Permanent deletion, recursive removal, or deletion outside the staged safe-root reversible-trash policy
- Network or device configuration changes
- Any Android device-control action beyond exact-stream volume

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
